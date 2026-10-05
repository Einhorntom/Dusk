using Microsoft.CommandPalette.Extensions;
using Shmuelie.WinRTServer;
using Shmuelie.WinRTServer.CsWinRT;

namespace Dispcontrol.CommandPalette;

public static class Program
{
    /// <summary>
    /// Command Palette starts this exe with -RegisterProcessAsComServer
    /// (see AppxManifest.xml) and talks to it over COM until it disposes the
    /// extension.
    /// </summary>
    [MTAThread]
    public static void Main(string[] args)
    {
        if (args.Length == 0 || args[0] != "-RegisterProcessAsComServer")
        {
            Console.WriteLine("This is the dispcontrol Command Palette extension; Command Palette starts it.");
            return;
        }
        using var disposed = new ManualResetEvent(false);
        var extension = new DispcontrolExtension(disposed);
        var server = new ComServer();
        server.RegisterClass<DispcontrolExtension, IExtension>(() => extension);
        server.Start();
        disposed.WaitOne();
        server.Stop();
        server.UnsafeDispose();
    }
}
