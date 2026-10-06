using System.Buffers.Binary;
using System.IO.Pipes;
using System.Text;
using System.Text.Json.Nodes;

namespace Dusk.Client;

/// <summary>A connected monitor, as listed by the daemon.</summary>
public sealed record MonitorInfo(string Id, string Name);

/// <summary>A control's current value. Enum controls carry their allowed values and names.</summary>
public sealed record Reading(
    string Control,
    int Value,
    string? ValueName,
    IReadOnlyList<int> EnumValues,
    IReadOnlyList<string?> EnumNames);

public sealed record PresetInfo(string Name, int Settings);

public sealed record ApplySummary(string Preset, int Applied, int Unchanged, int Skipped, int Failed);

/// <summary>The daemon (duskd) is not running.</summary>
public sealed class DaemonUnavailableException() : Exception("Dusk is not running");

/// <summary>The daemon refused a request; <see cref="Code"/> is the CLI exit code.</summary>
public sealed class DaemonException(int code, string message) : Exception(message)
{
    public int Code { get; } = code;
}

/// <summary>What the integrations need from the daemon (the IPC protocol, typed).</summary>
public interface IDaemon
{
    IReadOnlyList<MonitorInfo> ListMonitors();
    Reading Read(string monitor, string control);
    bool Write(string monitor, string control, int value, bool isEnum);
    IReadOnlyList<PresetInfo> ListPresets();
    ApplySummary ApplyPreset(string name);
}

/// <summary>
/// The daemon over its named pipe: one request per connection, each frame a
/// little-endian u32 length followed by UTF-8 JSON (see crates/ipc).
/// </summary>
public sealed class PipeDaemon : IDaemon
{
    public const string DefaultPipe = "dusk-v0";
    /// <summary>Same override as the daemon and the CLI.</summary>
    public const string PipeEnvironmentVariable = "DUSK_PIPE";
    private const int MaxMessageSize = 1_048_576;
    /// <summary>The request format this client speaks (crates/ipc PROTOCOL_VERSION).</summary>
    public const int ProtocolVersion = 1;

    /// <summary>
    /// How long to wait for an answer. Longer than the daemon's monitor
    /// timeout (6 s), short enough that PowerToys never looks frozen.
    /// </summary>
    public static readonly TimeSpan DefaultResponseTimeout = TimeSpan.FromSeconds(15);

    private readonly string pipeName;
    private readonly int connectTimeoutMs;
    private readonly TimeSpan responseTimeout;

    public PipeDaemon(string? pipeName = null, TimeSpan? connectTimeout = null, TimeSpan? responseTimeout = null)
    {
        var fromEnvironment = Environment.GetEnvironmentVariable(PipeEnvironmentVariable);
        this.pipeName = pipeName
            ?? (string.IsNullOrWhiteSpace(fromEnvironment) ? DefaultPipe : fromEnvironment.Trim());
        connectTimeoutMs = (int)(connectTimeout ?? TimeSpan.FromMilliseconds(500)).TotalMilliseconds;
        this.responseTimeout = responseTimeout ?? DefaultResponseTimeout;
    }

    public IReadOnlyList<MonitorInfo> ListMonitors() =>
        Send(new JsonObject { ["op"] = "list" })
            .AsArray()
            .Select(item => new MonitorInfo((string)item!["id"]!, (string)item["name"]!))
            .ToList();

    public Reading Read(string monitor, string control)
    {
        var result = Send(new JsonObject { ["op"] = "get", ["monitor"] = monitor, ["control"] = control });
        return new Reading(
            control,
            int.Parse((string)result["value"]!, System.Globalization.CultureInfo.InvariantCulture),
            (string?)result["value_name"],
            result["enum_values"]?.AsArray().Select(value => (int)value!).ToList() ?? [],
            result["enum_names"]?.AsArray().Select(name => (string?)name).ToList() ?? []);
    }

    public bool Write(string monitor, string control, int value, bool isEnum) =>
        (bool)Send(new JsonObject
        {
            ["op"] = "set",
            ["monitor"] = monitor,
            ["control"] = control,
            ["value"] = new JsonObject { ["kind"] = isEnum ? "enum" : "normalized", ["value"] = value },
        })["changed"]!;

    public IReadOnlyList<PresetInfo> ListPresets() =>
        Send(new JsonObject { ["op"] = "preset_list" })
            .AsArray()
            .Select(item => new PresetInfo((string)item!["name"]!, item["entries"]!.AsArray().Count))
            .ToList();

    public ApplySummary ApplyPreset(string name)
    {
        var result = Send(new JsonObject { ["op"] = "preset_apply", ["name"] = name });
        return new ApplySummary(
            (string)result["preset"]!,
            (int)result["applied"]!,
            (int)result["unchanged"]!,
            (int)result["skipped"]!,
            (int)result["failed"]!);
    }

    /// <summary>
    /// Sends one request and returns its <c>result</c>. Throws a
    /// <see cref="DaemonException"/> with code 4 if no answer arrives within
    /// the response timeout, e.g. while the daemon waits for the user to
    /// confirm an input change or for a monitor that does not answer.
    /// </summary>
    public JsonNode Send(JsonObject request)
    {
        // Asynchronous so the read can be cancelled; a synchronous pipe read cannot time out.
        using var pipe = new NamedPipeClientStream(".", pipeName, PipeDirection.InOut, PipeOptions.Asynchronous);
        try
        {
            pipe.Connect(connectTimeoutMs);
        }
        catch (TimeoutException)
        {
            throw new DaemonUnavailableException();
        }
        request["protocol"] = ProtocolVersion;
        using var deadline = new CancellationTokenSource(responseTimeout);
        byte[] answer;
        try
        {
            WriteFrame(pipe, Encoding.UTF8.GetBytes(request.ToJsonString()), deadline.Token);
            answer = ReadFrame(pipe, deadline.Token);
        }
        catch (OperationCanceledException)
        {
            throw new DaemonException(
                4,
                $"Dusk did not answer within {responseTimeout.TotalSeconds:0.#} s. It may be waiting for you to confirm an input change, or for a monitor that does not respond.");
        }
        var response = JsonNode.Parse(answer)
            ?? throw new DaemonException(1, "the daemon sent an empty response");
        if (response["ok"]?.GetValue<bool>() != true)
        {
            throw new DaemonException(
                (int?)response["code"] ?? 1,
                (string?)response["error"] ?? "the daemon returned an unspecified error");
        }
        return response["result"] ?? throw new DaemonException(1, "the daemon sent no result");
    }

    private static void WriteFrame(Stream pipe, byte[] payload, CancellationToken cancel)
    {
        if (payload.Length > MaxMessageSize)
        {
            throw new DaemonException(2, "request exceeds the 1 MiB limit");
        }
        var header = new byte[4];
        BinaryPrimitives.WriteUInt32LittleEndian(header, (uint)payload.Length);
        pipe.WriteAsync(header, cancel).AsTask().GetAwaiter().GetResult();
        pipe.WriteAsync(payload, cancel).AsTask().GetAwaiter().GetResult();
        pipe.FlushAsync(cancel).GetAwaiter().GetResult();
    }

    private static byte[] ReadFrame(Stream pipe, CancellationToken cancel)
    {
        var header = new byte[4];
        pipe.ReadExactlyAsync(header, cancel).AsTask().GetAwaiter().GetResult();
        var length = BinaryPrimitives.ReadUInt32LittleEndian(header);
        if (length > MaxMessageSize)
        {
            throw new DaemonException(1, "response exceeds the 1 MiB limit");
        }
        var payload = new byte[length];
        pipe.ReadExactlyAsync(payload, cancel).AsTask().GetAwaiter().GetResult();
        return payload;
    }
}
