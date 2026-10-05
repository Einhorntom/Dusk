using System.Runtime.InteropServices;
using Dispcontrol.Client;
using Microsoft.CommandPalette.Extensions;
using Microsoft.CommandPalette.Extensions.Toolkit;

namespace Dispcontrol.CommandPalette;

/// <summary>The COM class Command Palette creates; the Guid must match AppxManifest.xml.</summary>
[ComVisible(true)]
[Guid("4A0F6D2E-8C1B-4E57-9B3D-7F2A6C915E40")]
[ComDefaultInterface(typeof(IExtension))]
public sealed partial class DispcontrolExtension(ManualResetEvent disposed) : IExtension, IDisposable
{
    private readonly DispcontrolCommandsProvider provider = new(new PipeDaemon());

    public object? GetProvider(ProviderType providerType) =>
        providerType == ProviderType.Commands ? provider : null;

    public void Dispose() => disposed.Set();
}

/// <summary>
/// Top-level commands (SPEC-INT-2): the dispcontrol page, which accepts the
/// same commands as PowerToys Run's "dc", and every preset, so typing a preset
/// name in Command Palette finds it directly.
/// </summary>
public sealed partial class DispcontrolCommandsProvider : CommandProvider
{
    internal static readonly IconInfo AppIcon = IconHelpers.FromRelativePath("Assets\\Square44x44Logo.png");

    private readonly IDaemon daemon;
    private readonly DispcontrolPage page;

    public DispcontrolCommandsProvider(IDaemon daemon)
    {
        this.daemon = daemon;
        page = new DispcontrolPage(daemon);
        Id = "Dispcontrol";
        DisplayName = "dispcontrol";
        Icon = AppIcon;
    }

    public override ICommandItem[] TopLevelCommands()
    {
        var items = new List<ICommandItem>
        {
            new CommandItem(page)
            {
                Title = "dispcontrol",
                Subtitle = "Monitor presets, brightness, contrast, volume and input",
            },
        };
        try
        {
            items.AddRange(daemon.ListPresets().Select(preset => new CommandItem(
                new RunActionCommand(new ItemAction.ApplyPreset(preset.Name), daemon))
            {
                Title = $"Apply monitor preset: {preset.Name}",
                Subtitle = $"dispcontrol ({preset.Settings} settings)",
                Icon = AppIcon,
            }));
        }
        catch (Exception error) when (error is DaemonUnavailableException or DaemonException or IOException)
        {
            // The page explains that dispcontrol is not running.
        }
        return [.. items];
    }
}
