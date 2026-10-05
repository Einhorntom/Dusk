using System.Buffers.Binary;
using System.IO.Pipes;
using System.Text;
using System.Text.Json.Nodes;
using Dusk.Client;
using Xunit;

namespace Dusk.Client.Tests;

/// <summary>
/// The wire protocol against an in-process pipe server that answers like
/// duskd (crates/ipc): length-prefixed JSON, one request per connection.
/// </summary>
public class PipeDaemonTests
{
    private static string UniquePipe() => $"dusk-client-test-{Guid.NewGuid():N}";

    /// <summary>Serves one connection: records the request and sends <paramref name="response"/>.</summary>
    private static Task<JsonNode> ServeOnce(string pipe, string response) =>
        Task.Run(() =>
        {
            using var server = new NamedPipeServerStream(pipe, PipeDirection.InOut, 1);
            server.WaitForConnection();
            Span<byte> header = stackalloc byte[4];
            server.ReadExactly(header);
            var request = new byte[BinaryPrimitives.ReadUInt32LittleEndian(header)];
            server.ReadExactly(request);
            var payload = Encoding.UTF8.GetBytes(response);
            BinaryPrimitives.WriteUInt32LittleEndian(header, (uint)payload.Length);
            server.Write(header);
            server.Write(payload);
            server.Flush();
            return JsonNode.Parse(request)!;
        });

    [Fact]
    public async Task Requests_are_framed_json_and_results_are_parsed()
    {
        var pipe = UniquePipe();
        var served = ServeOnce(pipe, """
            {"ok":true,"result":{"control":"input","value":"49","value_name":"USB-C",
             "native_min":0,"native_max":0,"enum_values":[17,49],"enum_names":["HDMI 1","USB-C"]}}
            """);
        var reading = new PipeDaemon(pipe, TimeSpan.FromSeconds(5)).Read("L32p-30#0", "input");
        var request = await served;

        Assert.Equal("get", (string?)request["op"]);
        Assert.Equal("L32p-30#0", (string?)request["monitor"]);
        Assert.Equal(49, reading.Value);
        Assert.Equal("USB-C", reading.ValueName);
        Assert.Equal(new[] { 17, 49 }, reading.EnumValues);
        Assert.Equal(new[] { "HDMI 1", "USB-C" }, reading.EnumNames);
    }

    [Fact]
    public async Task Writes_send_the_value_kind_the_daemon_expects()
    {
        var pipe = UniquePipe();
        var served = ServeOnce(pipe, """{"ok":true,"result":{"changed":true}}""");
        Assert.True(new PipeDaemon(pipe, TimeSpan.FromSeconds(5)).Write("m", "brightness", 40, isEnum: false));
        var request = await served;
        Assert.Equal("normalized", (string?)request["value"]!["kind"]);
        Assert.Equal(40, (int)request["value"]!["value"]!);
    }

    [Fact]
    public async Task Daemon_errors_keep_their_exit_code()
    {
        var pipe = UniquePipe();
        var served = ServeOnce(pipe, """{"ok":false,"code":3,"error":"preset not found: Nope"}""");
        var error = Assert.Throws<DaemonException>(
            () => new PipeDaemon(pipe, TimeSpan.FromSeconds(5)).ApplyPreset("Nope"));
        await served;
        Assert.Equal(3, error.Code);
        Assert.Equal("preset not found: Nope", error.Message);
    }

    [Fact]
    public void A_missing_daemon_is_reported_as_not_running()
    {
        var daemon = new PipeDaemon(UniquePipe(), TimeSpan.FromMilliseconds(100));
        Assert.Throws<DaemonUnavailableException>(() => daemon.ListMonitors());
    }
}
