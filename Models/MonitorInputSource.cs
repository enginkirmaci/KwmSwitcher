namespace KwmSwitcher.Models;

public static class MonitorInputSource
{
    public const byte DisplayPort = 0x0F;
    public const byte Hdmi1 = 0x11;
    public const byte Hdmi2 = 0x12;
    public const byte Vga = 0x01;
    public const byte Dvi = 0x03;

    public const byte VcpCode = 0x60;

    public static string GetName(byte code) => code switch
    {
        DisplayPort => "DisplayPort",
        Hdmi1 => "HDMI-1",
        Hdmi2 => "HDMI-2",
        Vga => "VGA",
        Dvi => "DVI",
        _ => $"Unknown (0x{code:X2})"
    };
}
