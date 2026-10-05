using Dispcontrol.Client;

namespace Dispcontrol.Client.Tests;

/// <summary>An in-memory daemon: monitors with readings, presets, and a log of writes.</summary>
internal sealed class FakeDaemon : IDaemon
{
    public List<MonitorInfo> Monitors { get; } = [];
    public Dictionary<(string Monitor, string Control), Reading> Readings { get; } = [];
    public List<PresetInfo> Presets { get; } = [];
    public List<ControlWrite> Writes { get; } = [];
    public List<string> Applied { get; } = [];
    public bool Running { get; set; } = true;

    public static FakeDaemon Laptop()
    {
        var daemon = new FakeDaemon();
        daemon.Monitors.Add(new MonitorInfo("L32p-30#0", "L32p-30"));
        daemon.Monitors.Add(new MonitorInfo("Built-in-display", "Built-in display"));
        daemon.Readings[("L32p-30#0", "brightness")] = new Reading("brightness", 100, null, [], []);
        daemon.Readings[("L32p-30#0", "contrast")] = new Reading("contrast", 50, null, [], []);
        daemon.Readings[("L32p-30#0", "input")] =
            new Reading("input", 0x31, "USB-C", [0x0F, 0x11, 0x31], ["DisplayPort 1", "HDMI 1", "USB-C"]);
        daemon.Readings[("Built-in-display", "brightness")] = new Reading("brightness", 42, null, [], []);
        daemon.Presets.Add(new PresetInfo("Sunny day", 7));
        daemon.Presets.Add(new PresetInfo("Night mode", 7));
        return daemon;
    }

    private void EnsureRunning()
    {
        if (!Running)
        {
            throw new DaemonUnavailableException();
        }
    }

    public IReadOnlyList<MonitorInfo> ListMonitors()
    {
        EnsureRunning();
        return Monitors;
    }

    public Reading Read(string monitor, string control)
    {
        EnsureRunning();
        return Readings.TryGetValue((monitor, control), out var reading)
            ? reading
            : throw new DaemonException(2, $"monitor does not support {control}");
    }

    public bool Write(string monitor, string control, int value, bool isEnum)
    {
        EnsureRunning();
        Writes.Add(new ControlWrite(monitor, control, value, isEnum));
        return true;
    }

    public IReadOnlyList<PresetInfo> ListPresets()
    {
        EnsureRunning();
        return Presets;
    }

    public ApplySummary ApplyPreset(string name)
    {
        EnsureRunning();
        Applied.Add(name);
        return new ApplySummary(name, 3, 4, 0, 0);
    }
}
