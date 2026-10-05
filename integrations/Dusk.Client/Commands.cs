using System.Globalization;

namespace Dusk.Client;

/// <summary>What the user typed after the keyword (SPEC-INT-1).</summary>
public abstract record Command
{
    /// <summary>Preset names containing <paramref name="Filter"/> (empty: all).</summary>
    public sealed record Presets(string Filter) : Command;

    /// <summary><c>brightness 40</c>; without a value, shows the current levels.</summary>
    public sealed record Level(string Control, int? Value, string MonitorFilter) : Command;

    /// <summary><c>input hdmi</c>: inputs whose name contains the filter.</summary>
    public sealed record Input(string Filter) : Command;
}

public static class QueryParser
{
    public static readonly IReadOnlyList<string> LevelControls = ["brightness", "contrast", "volume"];

    /// <summary>
    /// <c>brightness 40 [monitor]</c>, <c>contrast</c>, <c>volume 20</c>, <c>input hdmi</c>, or a
    /// preset name. Control words may be shortened to three letters (<c>bri 40</c>).
    /// </summary>
    public static Command Parse(string search)
    {
        var words = search.Split(' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (words.Length == 0)
        {
            return new Command.Presets("");
        }
        var head = words[0].ToLowerInvariant();
        var level = head.Length >= 3
            ? LevelControls.FirstOrDefault(control => control.StartsWith(head, StringComparison.Ordinal))
            : null;
        if (level is not null)
        {
            var rest = words.Skip(1).ToList();
            int? value = null;
            if (rest.Count > 0 && int.TryParse(rest[0].TrimEnd('%'), NumberStyles.Integer, CultureInfo.InvariantCulture, out var number))
            {
                value = number;
                rest.RemoveAt(0);
            }
            return new Command.Level(level, value, string.Join(' ', rest));
        }
        if (head.Length >= 2 && "input".StartsWith(head, StringComparison.Ordinal))
        {
            return new Command.Input(string.Join(' ', words.Skip(1)));
        }
        return new Command.Presets(string.Join(' ', words));
    }
}

/// <summary>Something the user can pick; <see cref="Action"/> is null for information only.</summary>
public sealed record ResultItem(string Title, string Subtitle, ItemAction? Action);

public abstract record ItemAction
{
    public sealed record ApplyPreset(string Name) : ItemAction;

    public sealed record SetControl(IReadOnlyList<ControlWrite> Writes) : ItemAction;
}

public sealed record ControlWrite(string Monitor, string Control, int Value, bool IsEnum);

/// <summary>Builds the results for a command from the daemon's current state.</summary>
public static class ResultBuilder
{
    public const string Usage = "Type a preset name, brightness 40, contrast 60, volume 20 or input hdmi";

    public static IReadOnlyList<ResultItem> Build(Command command, IDaemon daemon)
    {
        try
        {
            return command switch
            {
                Command.Presets presets => BuildPresets(presets, daemon),
                Command.Level level => BuildLevel(level, daemon),
                Command.Input input => BuildInput(input, daemon),
                _ => [],
            };
        }
        catch (DaemonUnavailableException)
        {
            return [new ResultItem("Dusk is not running", "Start duskd to control your monitors.", null)];
        }
        catch (DaemonException error)
        {
            return [new ResultItem("Dusk could not answer", error.Message, null)];
        }
    }

    private static List<ResultItem> BuildPresets(Command.Presets command, IDaemon daemon)
    {
        var items = daemon
            .ListPresets()
            .Where(preset => Contains(preset.Name, command.Filter))
            .Select(preset => new ResultItem(
                preset.Name,
                $"Apply preset ({preset.Settings} settings)",
                new ItemAction.ApplyPreset(preset.Name)))
            .ToList();
        if (items.Count == 0)
        {
            items.Add(command.Filter.Length == 0
                ? new ResultItem("No presets yet", "Save one in Dusk Settings. " + Usage, null)
                : new ResultItem($"No preset matches \"{command.Filter}\"", Usage, null));
        }
        else if (command.Filter.Length == 0)
        {
            items.Add(new ResultItem("Dusk commands", Usage, null));
        }
        return items;
    }

    private static List<ResultItem> BuildLevel(Command.Level command, IDaemon daemon)
    {
        var title = Capitalize(command.Control);
        if (command.Value is < 0 or > 100)
        {
            return [new ResultItem($"{title} must be between 0 and 100", Usage, null)];
        }
        var readings = Supporting(daemon, command.Control, command.MonitorFilter);
        if (readings.Count == 0)
        {
            return [new ResultItem($"No monitor offers {command.Control}", "Check the monitor name or connection.", null)];
        }
        if (command.Value is not int value)
        {
            return readings
                .Select(item => new ResultItem(
                    $"{title} {item.Reading.Value}%",
                    $"{item.Monitor.Name} - type a value, e.g. {command.Control} 40",
                    null))
                .ToList();
        }
        var items = new List<ResultItem>();
        if (readings.Count > 1)
        {
            items.Add(new ResultItem(
                $"Set {command.Control} to {value}% on all monitors",
                string.Join(", ", readings.Select(item => $"{item.Monitor.Name} now {item.Reading.Value}%")),
                new ItemAction.SetControl(readings
                    .Select(item => new ControlWrite(item.Monitor.Id, command.Control, value, false))
                    .ToList())));
        }
        items.AddRange(readings.Select(item => new ResultItem(
            $"Set {command.Control} to {value}%",
            $"{item.Monitor.Name} - now {item.Reading.Value}%",
            new ItemAction.SetControl([new ControlWrite(item.Monitor.Id, command.Control, value, false)]))));
        return items;
    }

    private static List<ResultItem> BuildInput(Command.Input command, IDaemon daemon)
    {
        var items = new List<ResultItem>();
        foreach (var (monitor, reading) in Supporting(daemon, "input", ""))
        {
            for (var index = 0; index < reading.EnumValues.Count; index++)
            {
                var value = reading.EnumValues[index];
                var name = Name(value, index < reading.EnumNames.Count ? reading.EnumNames[index] : null);
                if (!Contains(name, command.Filter))
                {
                    continue;
                }
                var current = value == reading.Value;
                items.Add(new ResultItem(
                    current ? $"{name} (current input)" : $"Switch to {name}",
                    current
                        ? monitor.Name
                        : $"{monitor.Name} - asks for confirmation; may disconnect devices on the monitor's USB hub",
                    current ? null : new ItemAction.SetControl([new ControlWrite(monitor.Id, "input", value, true)])));
            }
        }
        if (items.Count == 0)
        {
            items.Add(new ResultItem($"No input matches \"{command.Filter}\"", "Type input and part of a name, e.g. input hdmi", null));
        }
        return items;
    }

    /// <summary>Monitors (matching <paramref name="monitorFilter"/>) that offer the control, with its reading.</summary>
    private static List<(MonitorInfo Monitor, Reading Reading)> Supporting(IDaemon daemon, string control, string monitorFilter)
    {
        var result = new List<(MonitorInfo, Reading)>();
        foreach (var monitor in daemon.ListMonitors())
        {
            if (!Contains(monitor.Name, monitorFilter) && !Contains(monitor.Id, monitorFilter))
            {
                continue;
            }
            try
            {
                result.Add((monitor, daemon.Read(monitor.Id, control)));
            }
            catch (DaemonException error) when (error.Code is 2 or 3 or 4)
            {
                // Unsupported (2), gone (3) or not responding (4): leave it out.
            }
        }
        return result;
    }

    private static string Name(int value, string? name) =>
        name ?? string.Create(CultureInfo.InvariantCulture, $"raw-0x{value:X2}");

    private static bool Contains(string text, string filter) =>
        filter.Length == 0 || text.Contains(filter, StringComparison.OrdinalIgnoreCase);

    private static string Capitalize(string text) =>
        text.Length == 0 ? text : char.ToUpperInvariant(text[0]) + text[1..];
}

/// <summary>Runs a picked result and describes what happened.</summary>
public static class ActionRunner
{
    public static string Run(ItemAction action, IDaemon daemon)
    {
        try
        {
            switch (action)
            {
                case ItemAction.ApplyPreset preset:
                    var report = daemon.ApplyPreset(preset.Name);
                    return report.Failed == 0
                        ? $"Applied {report.Preset}: {report.Applied} changed, {report.Unchanged} already set"
                        : $"Applied {report.Preset} with {report.Failed} failed setting(s)";
                case ItemAction.SetControl set:
                    var failures = new List<string>();
                    foreach (var write in set.Writes)
                    {
                        try
                        {
                            daemon.Write(write.Monitor, write.Control, write.Value, write.IsEnum);
                        }
                        catch (DaemonException error)
                        {
                            failures.Add($"{write.Monitor}: {error.Message}");
                        }
                    }
                    return failures.Count == 0 ? "Done" : string.Join("; ", failures);
                default:
                    return "Nothing to do";
            }
        }
        catch (DaemonUnavailableException error)
        {
            return error.Message;
        }
        catch (DaemonException error)
        {
            return error.Message;
        }
    }
}
