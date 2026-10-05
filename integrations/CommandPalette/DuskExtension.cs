using System.Runtime.InteropServices;
using Dusk.Client;
using Microsoft.CommandPalette.Extensions;
using Microsoft.CommandPalette.Extensions.Toolkit;

namespace Dusk.CommandPalette;

/// <summary>The COM class Command Palette creates; the Guid must match AppxManifest.xml.</summary>
[ComVisible(true)]
[Guid("EADDD729-27FC-467A-BEC1-19F2BB265C72")]
[ComDefaultInterface(typeof(IExtension))]
public sealed partial class DuskExtension(ManualResetEvent disposed) : IExtension, IDisposable
{
    private readonly DuskCommandsProvider provider = new(new PipeDaemon());

    public object? GetProvider(ProviderType providerType) =>
        providerType == ProviderType.Commands ? provider : null;

    public void Dispose() => disposed.Set();
}

/// <summary>
/// Top-level commands (SPEC-INT-2): the Dusk page, which accepts the
/// same commands as PowerToys Run's "dusk", and every preset, so typing a preset
/// name in Command Palette finds it directly.
/// </summary>
public sealed partial class DuskCommandsProvider : CommandProvider
{
    internal static readonly IconInfo AppIcon = IconHelpers.FromRelativePath("Assets\\Square44x44Logo.png");

    private readonly IDaemon daemon;
    private readonly DuskPage page;

    public DuskCommandsProvider(IDaemon daemon)
    {
        this.daemon = daemon;
        page = new DuskPage(daemon);
        Id = "Dusk";
        DisplayName = "Dusk";
        Icon = AppIcon;
    }

    public override ICommandItem[] TopLevelCommands()
    {
        var items = new List<ICommandItem>
        {
            new CommandItem(page)
            {
                Title = "Dusk",
                Subtitle = "Monitor presets, brightness, contrast, volume and input",
            },
        };
        try
        {
            items.AddRange(daemon.ListPresets().Select(preset => new CommandItem(
                new RunActionCommand(new ItemAction.ApplyPreset(preset.Name), daemon))
            {
                Title = $"Apply monitor preset: {preset.Name}",
                Subtitle = $"Dusk ({preset.Settings} settings)",
                Icon = AppIcon,
            }));
        }
        catch (Exception error) when (error is DaemonUnavailableException or DaemonException or IOException)
        {
            // The page explains that Dusk is not running.
        }
        return [.. items];
    }
}
