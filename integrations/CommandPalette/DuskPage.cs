using Dusk.Client;
using Microsoft.CommandPalette.Extensions;
using Microsoft.CommandPalette.Extensions.Toolkit;

namespace Dusk.CommandPalette;

/// <summary>
/// Searchable page with the same commands as PowerToys Run's "dusk": preset
/// names, brightness 40, contrast 60, volume 20, input hdmi (SPEC-INT-2).
/// </summary>
internal sealed partial class DuskPage : DynamicListPage
{
    private readonly IDaemon daemon;

    public DuskPage(IDaemon daemon)
    {
        this.daemon = daemon;
        Icon = DuskCommandsProvider.AppIcon;
        Title = "Dusk";
        Name = "Open";
        PlaceholderText = ResultBuilder.Usage;
    }

    public override void UpdateSearchText(string oldSearch, string newSearch) => RaiseItemsChanged();

    public override IListItem[] GetItems() =>
        [.. ResultBuilder
            .Build(QueryParser.Parse(SearchText ?? ""), daemon)
            .Select(item => new ListItem(item.Action is null
                ? new NoOpCommand()
                : new RunActionCommand(item.Action, daemon))
            {
                Title = item.Title,
                Subtitle = item.Subtitle,
                Icon = DuskCommandsProvider.AppIcon,
            })];
}

/// <summary>Runs a result's action through the daemon.</summary>
internal sealed partial class RunActionCommand : InvokableCommand
{
    private readonly ItemAction action;
    private readonly IDaemon daemon;

    public RunActionCommand(ItemAction action, IDaemon daemon)
    {
        this.action = action;
        this.daemon = daemon;
        Name = action is ItemAction.ApplyPreset ? "Apply" : "Set";
    }

    public override ICommandResult Invoke()
    {
        // An input change waits for the daemon's confirmation dialog; don't
        // keep Command Palette waiting for it.
        var changesInput = action is ItemAction.SetControl set
            && set.Writes.Any(write => write.Control == "input");
        if (changesInput)
        {
            _ = Task.Run(() => ActionRunner.Run(action, daemon));
            return CommandResult.Dismiss();
        }
        return CommandResult.ShowToast(ActionRunner.Run(action, daemon));
    }
}
