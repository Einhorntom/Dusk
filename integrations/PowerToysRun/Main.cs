using Dispcontrol.Client;
using ManagedCommon;
using Wox.Plugin;

namespace Dispcontrol.PowerToysRun;

/// <summary>
/// PowerToys Run plugin, keyword <c>dc</c> (SPEC-INT-1). Results and actions
/// come from Dispcontrol.Client; all rules stay in the daemon (SPEC-INT-3).
/// </summary>
public sealed class Main : IPlugin, IDisposable
{
    /// <summary>Must match the ID in plugin.json.</summary>
    public static string PluginID => "6E1F3B9A0C4D4F2B9D7A5C8E2B1F4A3D";

    private readonly IDaemon daemon;
    private PluginInitContext? context;
    private string iconPath = "Images\\dispcontrol.dark.png";

    public Main()
        : this(new PipeDaemon())
    {
    }

    internal Main(IDaemon daemon)
    {
        this.daemon = daemon;
    }

    public string Name => "dispcontrol";

    public string Description => "Apply monitor presets and set brightness, contrast, volume or input.";

    public void Init(PluginInitContext context)
    {
        this.context = context;
        context.API.ThemeChanged += OnThemeChanged;
        UpdateIcon(context.API.GetCurrentTheme());
    }

    public List<Result> Query(Query query)
    {
        var items = ResultBuilder.Build(QueryParser.Parse(query?.Search ?? ""), daemon);
        return items
            .Select((item, index) => new Result
            {
                Title = item.Title,
                SubTitle = item.Subtitle,
                IcoPath = iconPath,
                // Keep the builder's order.
                Score = items.Count - index,
                Action = _ => Run(item.Action),
            })
            .ToList();
    }

    /// <summary>
    /// Runs the action off the UI thread: an input change waits for the
    /// daemon's confirmation dialog, which must not freeze PowerToys Run.
    /// Returns false for information-only results so the window stays open.
    /// </summary>
    private bool Run(ItemAction? action)
    {
        if (action is null)
        {
            return false;
        }
        _ = Task.Run(() =>
        {
            var message = ActionRunner.Run(action, daemon);
            if (message != "Done" && !message.StartsWith("Applied ", StringComparison.Ordinal))
            {
                context?.API.ShowMsg("dispcontrol", message, iconPath);
            }
        });
        return true;
    }

    private void OnThemeChanged(Theme oldTheme, Theme newTheme) => UpdateIcon(newTheme);

    private void UpdateIcon(Theme theme) =>
        iconPath = theme is Theme.Light or Theme.HighContrastWhite
            ? "Images\\dispcontrol.light.png"
            : "Images\\dispcontrol.dark.png";

    public void Dispose()
    {
        if (context is not null)
        {
            context.API.ThemeChanged -= OnThemeChanged;
        }
    }
}
