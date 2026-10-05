using Dusk.Client;
using Xunit;

namespace Dusk.Client.Tests;

public class QueryParserTests
{
    [Theory]
    [InlineData("", "")]
    [InlineData("  night ", "night")]
    [InlineData("Sunny day", "Sunny day")]
    public void Plain_text_searches_presets(string search, string filter)
    {
        Assert.Equal(new Command.Presets(filter), QueryParser.Parse(search));
    }

    [Fact]
    public void Level_commands_take_an_optional_value_and_monitor_filter()
    {
        Assert.Equal(new Command.Level("brightness", 40, ""), QueryParser.Parse("brightness 40"));
        Assert.Equal(new Command.Level("brightness", 40, "built-in"), QueryParser.Parse("bri 40% built-in"));
        Assert.Equal(new Command.Level("contrast", null, ""), QueryParser.Parse("Contrast"));
        Assert.Equal(new Command.Level("volume", null, "l32p"), QueryParser.Parse("vol l32p"));
        // Two letters are not enough to mean a level control.
        Assert.Equal(new Command.Presets("br 40"), QueryParser.Parse("br 40"));
    }

    [Fact]
    public void Input_commands_filter_by_name()
    {
        Assert.Equal(new Command.Input("hdmi"), QueryParser.Parse("input hdmi"));
        Assert.Equal(new Command.Input("display port"), QueryParser.Parse("inp display port"));
        Assert.Equal(new Command.Input(""), QueryParser.Parse("in"));
    }
}

public class ResultBuilderTests
{
    private static IReadOnlyList<ResultItem> Results(FakeDaemon daemon, string search) =>
        ResultBuilder.Build(QueryParser.Parse(search), daemon);

    private static IReadOnlyList<ControlWrite> Writes(ItemAction? action) =>
        Assert.IsType<ItemAction.SetControl>(action).Writes;

    [Fact]
    public void Presets_are_listed_and_filtered_and_picking_one_applies_it()
    {
        var daemon = FakeDaemon.Laptop();
        var all = Results(daemon, "");
        Assert.Equal(new[] { "Sunny day", "Night mode", "Dusk commands" }, all.Select(item => item.Title));
        var night = Assert.Single(Results(daemon, "NIGHT"));
        Assert.Equal(new ItemAction.ApplyPreset("Night mode"), night.Action);

        Assert.StartsWith("Applied Night mode", ActionRunner.Run(night.Action!, daemon));
        Assert.Equal(new[] { "Night mode" }, daemon.Applied);
    }

    [Fact]
    public void Unknown_text_and_an_empty_preset_list_explain_the_commands()
    {
        var daemon = FakeDaemon.Laptop();
        var none = Assert.Single(Results(daemon, "zzz"));
        Assert.Null(none.Action);
        Assert.Equal(ResultBuilder.Usage, none.Subtitle);
        daemon.Presets.Clear();
        Assert.Equal("No presets yet", Assert.Single(Results(daemon, "")).Title);
    }

    [Fact]
    public void A_level_with_a_value_offers_every_monitor_and_all_of_them_together()
    {
        var daemon = FakeDaemon.Laptop();
        var items = Results(daemon, "brightness 40");
        Assert.Equal(3, items.Count);
        Assert.Equal("Set brightness to 40% on all monitors", items[0].Title);
        Assert.Contains("L32p-30 now 100%", items[0].Subtitle);
        Assert.Contains("Built-in display now 42%", items[0].Subtitle);

        ActionRunner.Run(items[0].Action!, daemon);
        Assert.Equal(
            new[]
            {
                new ControlWrite("L32p-30#0", "brightness", 40, false),
                new ControlWrite("Built-in-display", "brightness", 40, false),
            },
            daemon.Writes);
    }

    [Fact]
    public void A_level_only_lists_monitors_that_offer_it_and_match_the_filter()
    {
        var daemon = FakeDaemon.Laptop();
        var contrast = Assert.Single(Results(daemon, "contrast 60"));
        Assert.Equal("L32p-30 - now 50%", contrast.Subtitle);
        var builtIn = Assert.Single(Results(daemon, "brightness 30 built"));
        Assert.Equal(new[] { new ControlWrite("Built-in-display", "brightness", 30, false) }, Writes(builtIn.Action));
        Assert.Equal("No monitor offers volume", Assert.Single(Results(daemon, "volume 20")).Title);
    }

    [Fact]
    public void A_level_without_a_value_shows_the_current_values_only()
    {
        var items = Results(FakeDaemon.Laptop(), "brightness");
        Assert.Equal(new[] { "Brightness 100%", "Brightness 42%" }, items.Select(item => item.Title));
        Assert.All(items, item => Assert.Null(item.Action));
    }

    [Theory]
    [InlineData("brightness 101")]
    [InlineData("brightness -1")]
    public void Out_of_range_levels_are_refused_without_an_action(string search)
    {
        var item = Assert.Single(Results(FakeDaemon.Laptop(), search));
        Assert.Null(item.Action);
        Assert.Contains("between 0 and 100", item.Title);
    }

    [Fact]
    public void Inputs_are_listed_by_name_and_the_current_one_is_not_actionable()
    {
        var daemon = FakeDaemon.Laptop();
        var items = Results(daemon, "input");
        Assert.Equal(
            new[] { "Switch to DisplayPort 1", "Switch to HDMI 1", "USB-C (current input)" },
            items.Select(item => item.Title));
        Assert.Null(items[2].Action);
        Assert.Contains("asks for confirmation", items[1].Subtitle);

        var hdmi = Assert.Single(Results(daemon, "input hdmi"));
        ActionRunner.Run(hdmi.Action!, daemon);
        Assert.Equal(new[] { new ControlWrite("L32p-30#0", "input", 0x11, true) }, daemon.Writes);
    }

    [Fact]
    public void A_stopped_daemon_is_reported_and_nothing_else_happens()
    {
        var daemon = FakeDaemon.Laptop();
        daemon.Running = false;
        foreach (var search in new[] { "", "brightness 40", "input" })
        {
            var item = Assert.Single(Results(daemon, search));
            Assert.Equal("Dusk is not running", item.Title);
            Assert.Null(item.Action);
        }
        Assert.Equal("Dusk is not running", ActionRunner.Run(new ItemAction.ApplyPreset("Night mode"), daemon));
    }
}
