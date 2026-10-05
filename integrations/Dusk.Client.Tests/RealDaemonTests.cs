using System.Diagnostics;
using Dusk.Client;
using Xunit;

namespace Dusk.Client.Tests;

/// <summary>Runs only when DUSKD_EXE names a built duskd; otherwise reported as skipped.</summary>
public sealed class RealDaemonFactAttribute : FactAttribute
{
    public const string Variable = "DUSKD_EXE";

    public RealDaemonFactAttribute()
    {
        if (!File.Exists(Environment.GetEnvironmentVariable(Variable)))
        {
            Skip = $"set {Variable} to a built duskd.exe to run against the real daemon";
        }
    }
}

/// <summary>
/// Cross-language contract: the C# client and result builder against the real
/// Rust daemon with simulated monitors (--demo), on a private pipe and config.
/// </summary>
public sealed class RealDaemonTests : IDisposable
{
    private readonly string pipe = $"dusk-cs-contract-{Guid.NewGuid():N}";
    private readonly string folder = Path.Combine(Path.GetTempPath(), $"dusk-cs-contract-{Guid.NewGuid():N}");
    private Process? process;

    private PipeDaemon StartDaemon()
    {
        Directory.CreateDirectory(folder);
        var start = new ProcessStartInfo(Environment.GetEnvironmentVariable(RealDaemonFactAttribute.Variable)!)
        {
            UseShellExecute = false,
        };
        foreach (var argument in new[] { "--demo", "--background", "--config", Path.Combine(folder, "config.toml") })
        {
            start.ArgumentList.Add(argument);
        }
        start.Environment[PipeDaemon.PipeEnvironmentVariable] = pipe;
        process = Process.Start(start);
        var daemon = new PipeDaemon(pipe, TimeSpan.FromMilliseconds(200));
        var deadline = DateTime.UtcNow.AddSeconds(20);
        while (true)
        {
            try
            {
                daemon.ListMonitors();
                return daemon;
            }
            catch (DaemonUnavailableException) when (DateTime.UtcNow < deadline)
            {
                Thread.Sleep(50);
            }
        }
    }

    [RealDaemonFact]
    public void The_client_and_results_work_against_the_real_daemon()
    {
        var daemon = StartDaemon();
        Assert.Equal(
            new[] { "Demo-monitor", "Built-in-display" },
            daemon.ListMonitors().Select(monitor => monitor.Id));

        var all = ResultBuilder.Build(QueryParser.Parse("brightness 40"), daemon);
        Assert.Equal("Set brightness to 40% on all monitors", all[0].Title);
        Assert.Equal("Done", ActionRunner.Run(all[0].Action!, daemon));
        Assert.Equal(40, daemon.Read("Demo-monitor", "brightness").Value);
        Assert.Equal(40, daemon.Read("Built-in-display", "brightness").Value);

        var input = daemon.Read("Demo-monitor", "input");
        Assert.Equal("USB-C", input.ValueName);
        Assert.Contains("HDMI 1", input.EnumNames);
        Assert.Single(ResultBuilder.Build(QueryParser.Parse("input hdmi"), daemon));

        Assert.Equal("No presets yet", Assert.Single(ResultBuilder.Build(QueryParser.Parse(""), daemon)).Title);
        Assert.Equal(3, Assert.Throws<DaemonException>(() => daemon.ApplyPreset("Missing")).Code);
    }

    public void Dispose()
    {
        if (process is { HasExited: false })
        {
            process.Kill();
            process.WaitForExit();
        }
        process?.Dispose();
        try
        {
            Directory.Delete(folder, recursive: true);
        }
        catch (DirectoryNotFoundException)
        {
        }
    }
}
