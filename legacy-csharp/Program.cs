using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Windows.Forms;
using Microsoft.Win32;

namespace CornerPin;

internal static class Program
{
    private static Mutex? _mutex;

    [STAThread]
    private static void Main()
    {
        ApplicationConfiguration.Initialize();
        _mutex = new Mutex(true, @"Local\CornerPinSingleInstance", out bool createdNew);
        if (!createdNew)
        {
            MessageBox.Show(
                "CornerPin zaten çalışıyor. Sağ altta saatin yanındaki tepsi simgesine bak; gizliyse ^ okuna tıkla.",
                "CornerPin", MessageBoxButtons.OK, MessageBoxIcon.Information);
            return;
        }
        Application.Run(new CornerPinContext());
        GC.KeepAlive(_mutex);
    }
}

public enum Corner { TopLeft, TopRight, BottomLeft, BottomRight }

public enum SizeMode { Keep, Tall420, Tall520 }

public class Settings
{
    public string WindowMatch { get; set; } = "Telegram";
    public Corner Corner { get; set; } = Corner.BottomRight;
    public SizeMode SizeMode { get; set; } = SizeMode.Keep;
    public bool TopMost { get; set; } = true;
    public bool RunAtStartup { get; set; } = false;
}

internal static class Win32
{
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hWnd, int nCmdShow);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr hWnd, int nIndex);

    public const int SW_RESTORE = 9;
    public const int GWL_EXSTYLE = -20;
    public const int WS_EX_TOPMOST = 0x0008;
    public const int WS_EX_TOOLWINDOW = 0x0080;
    public static readonly IntPtr HWND_TOPMOST = new(-1);
    public static readonly IntPtr HWND_NOTOPMOST = new(-2);
    public const uint SWP_NOACTIVATE = 0x0010;
    public const uint SWP_NOOWNERZORDER = 0x0200;
    public const uint SWP_SHOWWINDOW = 0x0040;

    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }

    public static string GetTitle(IntPtr hWnd)
    {
        var sb = new StringBuilder(256);
        GetWindowText(hWnd, sb, sb.Capacity);
        return sb.ToString();
    }

    public static List<(IntPtr Hwnd, string Title)> ListWindows()
    {
        var list = new List<(IntPtr, string)>();
        EnumWindows((h, _) =>
        {
            if (!IsWindowVisible(h)) return true;
            var ex = GetWindowLong(h, GWL_EXSTYLE);
            if ((ex & WS_EX_TOOLWINDOW) == WS_EX_TOOLWINDOW) return true;
            var t = GetTitle(h);
            if (!string.IsNullOrWhiteSpace(t)) list.Add((h, t));
            return true;
        }, IntPtr.Zero);
        return list;
    }
}

internal sealed class CornerPinContext : ApplicationContext
{
    private static readonly JsonSerializerOptions JsonOpts = new()
    {
        WriteIndented = true,
        Converters = { new JsonStringEnumConverter() }
    };

    private readonly NotifyIcon _tray;
    private readonly System.Windows.Forms.Timer _timer;
    private readonly ContextMenuStrip _menu;
    private readonly ToolStripMenuItem _windowMenu = new("Pencere");
    private readonly ToolStripMenuItem _cornerMenu = new("Köşe");
    private readonly ToolStripMenuItem _sizeMenu = new("Boyut");
    private readonly ToolStripMenuItem _topMostItem;
    private readonly ToolStripMenuItem _startupItem;
    private readonly string _settingsPath;
    private Settings _settings;
    private IntPtr _hwnd = IntPtr.Zero;
    private Size _keepSize = Size.Empty;
    private bool _notified;

    public CornerPinContext()
    {
        var dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "CornerPin");
        Directory.CreateDirectory(dir);
        _settingsPath = Path.Combine(dir, "settings.json");
        _settings = LoadSettings();
        ApplyStartup(_settings.RunAtStartup);

        _menu = new ContextMenuStrip();
        _windowMenu.DropDownOpening += FillWindowMenu;
        BuildCornerMenu();
        BuildSizeMenu();

        _topMostItem = new ToolStripMenuItem("Her Zaman Üstte")
        {
            Checked = _settings.TopMost,
            CheckOnClick = true
        };
        _topMostItem.CheckedChanged += (_, _) => { _settings.TopMost = _topMostItem.Checked; SaveSettings(); Pin(); };

        _startupItem = new ToolStripMenuItem("Windows İle Başlat")
        {
            Checked = _settings.RunAtStartup,
            CheckOnClick = true
        };
        _startupItem.CheckedChanged += (_, _) => { _settings.RunAtStartup = _startupItem.Checked; SaveSettings(); ApplyStartup(_startupItem.Checked); };

        _menu.Items.Add(_windowMenu);
        _menu.Items.Add(_cornerMenu);
        _menu.Items.Add(_sizeMenu);
        _menu.Items.Add(new ToolStripSeparator());
        _menu.Items.Add(_topMostItem);
        _menu.Items.Add(_startupItem);
        _menu.Items.Add(new ToolStripSeparator());
        _menu.Items.Add(new ToolStripMenuItem("Şimdi Sabitle", null, (_, _) => Pin()));
        _menu.Items.Add(new ToolStripMenuItem("Çıkış", null, (_, _) => ExitApp()));

        _tray = new NotifyIcon
        {
            Icon = CreateIcon(),
            Text = "CornerPin",
            ContextMenuStrip = _menu,
            Visible = true
        };
        _tray.DoubleClick += (_, _) => Pin();

        _timer = new System.Windows.Forms.Timer { Interval = 400 };
        _timer.Tick += (_, _) => Pin();
        _timer.Start();

        // İlk açılışta "çalışıyorum" balonu (mesaj döngüsü başlayınca göster)
        var hello = new System.Windows.Forms.Timer { Interval = 600 };
        hello.Tick += (_, _) =>
        {
            hello.Stop();
            hello.Dispose();
            _tray.ShowBalloonTip(5000, "CornerPin",
                "Çalışıyorum, beni tepside bulacaksın. Telegram açıksa birazdan köşeye sabitlenecek.",
                ToolTipIcon.Info);
        };
        hello.Start();
    }

    private void BuildCornerMenu()
    {
        AddCornerItem(Corner.BottomRight, "Sağ Alt");
        AddCornerItem(Corner.TopRight, "Sağ Üst");
        AddCornerItem(Corner.BottomLeft, "Sol Alt");
        AddCornerItem(Corner.TopLeft, "Sol Üst");
    }

    private void AddCornerItem(Corner corner, string name)
    {
        var item = new ToolStripMenuItem(name) { Tag = corner, Checked = _settings.Corner == corner };
        item.Click += (_, _) => { _settings.Corner = corner; SaveSettings(); RefreshChecks(); Pin(); };
        _cornerMenu.DropDownItems.Add(item);
    }

    private void BuildSizeMenu()
    {
        AddSizeItem(SizeMode.Keep, "Mevcut Boyut");
        AddSizeItem(SizeMode.Tall420, "Tam Yükseklik · 420 px");
        AddSizeItem(SizeMode.Tall520, "Tam Yükseklik · 520 px");
    }

    private void AddSizeItem(SizeMode mode, string name)
    {
        var item = new ToolStripMenuItem(name) { Tag = mode, Checked = _settings.SizeMode == mode };
        item.Click += (_, _) => { _settings.SizeMode = mode; _keepSize = Size.Empty; SaveSettings(); RefreshChecks(); Pin(); };
        _sizeMenu.DropDownItems.Add(item);
    }

    private void RefreshChecks()
    {
        foreach (ToolStripMenuItem item in _cornerMenu.DropDownItems)
            item.Checked = item.Tag is Corner c && c == _settings.Corner;
        foreach (ToolStripMenuItem item in _sizeMenu.DropDownItems)
            item.Checked = item.Tag is SizeMode m && m == _settings.SizeMode;
    }

    private void FillWindowMenu(object? sender, EventArgs e)
    {
        _windowMenu.DropDownItems.Clear();
        var auto = new ToolStripMenuItem("Telegram (Otomatik)")
        {
            Checked = string.Equals(_settings.WindowMatch, "Telegram", StringComparison.OrdinalIgnoreCase)
        };
        auto.Click += (_, _) => SelectWindow("Telegram");
        _windowMenu.DropDownItems.Add(auto);
        _windowMenu.DropDownItems.Add(new ToolStripSeparator());

        foreach (var w in Win32.ListWindows().Take(40))
        {
            var full = w.Title;
            var shortTitle = full.Length > 50 ? full[..50] + "…" : full;
            var item = new ToolStripMenuItem(shortTitle)
            {
                Checked = string.Equals(_settings.WindowMatch, full, StringComparison.OrdinalIgnoreCase)
            };
            item.Click += (_, _) => SelectWindow(full);
            _windowMenu.DropDownItems.Add(item);
        }
    }

    private void SelectWindow(string title)
    {
        _settings.WindowMatch = title;
        _hwnd = IntPtr.Zero;
        _keepSize = Size.Empty;
        _notified = false;
        SaveSettings();
        Pin();
    }

    private void Pin()
    {
        if (_hwnd == IntPtr.Zero || !Win32.IsWindow(_hwnd))
        {
            _hwnd = FindTarget();
            _keepSize = Size.Empty;
            if (_hwnd == IntPtr.Zero)
            {
                _tray.Text = "CornerPin · Bekleniyor: " + Truncate(_settings.WindowMatch, 20);
                return;
            }
        }

        // Win + D / Win + M sonrası geri getir
        if (Win32.IsIconic(_hwnd)) Win32.ShowWindow(_hwnd, Win32.SW_RESTORE);
        if (Win32.IsZoomed(_hwnd)) Win32.ShowWindow(_hwnd, Win32.SW_RESTORE);

        Win32.GetWindowRect(_hwnd, out var r);
        if (_keepSize.IsEmpty)
            _keepSize = new Size(Math.Max(200, r.Right - r.Left), Math.Max(200, r.Bottom - r.Top));

        var wa = Screen.FromHandle(_hwnd).WorkingArea;
        int w = _keepSize.Width, h = _keepSize.Height;
        switch (_settings.SizeMode)
        {
            case SizeMode.Tall420: w = 420; h = wa.Height; break;
            case SizeMode.Tall520: w = 520; h = wa.Height; break;
        }

        var (x, y) = _settings.Corner switch
        {
            Corner.TopLeft => (wa.Left, wa.Top),
            Corner.TopRight => (wa.Right - w, wa.Top),
            Corner.BottomLeft => (wa.Left, wa.Bottom - h),
            _ => (wa.Right - w, wa.Bottom - h)
        };

        bool isTopMost = (Win32.GetWindowLong(_hwnd, Win32.GWL_EXSTYLE) & Win32.WS_EX_TOPMOST) != 0;
        bool wantTop = _settings.TopMost;
        bool posDiff = Math.Abs(r.Left - x) > 1 || Math.Abs(r.Top - y) > 1
            || Math.Abs((r.Right - r.Left) - w) > 1 || Math.Abs((r.Bottom - r.Top) - h) > 1;

        if (posDiff || isTopMost != wantTop)
        {
            Win32.SetWindowPos(_hwnd, wantTop ? Win32.HWND_TOPMOST : Win32.HWND_NOTOPMOST,
                x, y, w, h, Win32.SWP_NOACTIVATE | Win32.SWP_NOOWNERZORDER | Win32.SWP_SHOWWINDOW);
        }

        if (!_notified)
        {
            _notified = true;
            _tray.ShowBalloonTip(3000, "CornerPin", $"{_settings.WindowMatch} {CornerName()} köşesine sabitlendi.", ToolTipIcon.Info);
        }
        _tray.Text = "CornerPin · " + Truncate(Win32.GetTitle(_hwnd), 30);
    }

    private string CornerName() => _settings.Corner switch
    {
        Corner.TopLeft => "Sol Üst",
        Corner.TopRight => "Sağ Üst",
        Corner.BottomLeft => "Sol Alt",
        _ => "Sağ Alt"
    };

    private IntPtr FindTarget()
    {
        var match = _settings.WindowMatch;
        var wins = Win32.ListWindows();
        var exact = wins.FirstOrDefault(w => string.Equals(w.Title, match, StringComparison.OrdinalIgnoreCase));
        if (exact.Hwnd != IntPtr.Zero) return exact.Hwnd;
        var partial = wins.FirstOrDefault(w => w.Title.Contains(match, StringComparison.OrdinalIgnoreCase));
        return partial.Hwnd;
    }

    private Settings LoadSettings()
    {
        try
        {
            if (File.Exists(_settingsPath))
                return JsonSerializer.Deserialize<Settings>(File.ReadAllText(_settingsPath), JsonOpts) ?? new Settings();
        }
        catch { }
        return new Settings();
    }

    private void SaveSettings()
    {
        try { File.WriteAllText(_settingsPath, JsonSerializer.Serialize(_settings, JsonOpts)); } catch { }
    }

    private static void ApplyStartup(bool enable)
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(@"SOFTWARE\Microsoft\Windows\CurrentVersion\Run", true);
            if (key == null) return;
            if (enable) key.SetValue("CornerPin", $"\"{Application.ExecutablePath}\"");
            else key.DeleteValue("CornerPin", false);
        }
        catch { }
    }

    private static string Truncate(string s, int max) => s.Length <= max ? s : s[..max];

    private static Icon CreateIcon()
    {
        var bmp = new Bitmap(32, 32);
        using (var g = Graphics.FromImage(bmp))
        {
            g.Clear(Color.FromArgb(24, 26, 32));
            using var frame = new Pen(Color.FromArgb(90, 160, 255), 2);
            g.DrawRectangle(frame, 1, 1, 29, 29);
            using var brush = new SolidBrush(Color.FromArgb(255, 150, 60));
            g.FillRectangle(brush, 17, 17, 13, 13);
        }
        return Icon.FromHandle(bmp.GetHicon());
    }

    private void ExitApp()
    {
        _timer.Stop();
        _tray.Visible = false;
        Application.Exit();
    }
}
