namespace KwmSwitcher.Models;

public static class MonitorInputSource
{
    public const byte DisplayPort = 0x0F;
    public const byte Hdmi1 = 0x11;
    public const byte Hdmi2 = 0x12;
    public const byte Vga = 0x01;
    public const byte Dvi = 0x03;
    public const byte UsbC = 0x20;

    public const byte VcpCode = 0x60;

    public static string GetName(byte code) => code switch
    {
        DisplayPort => "DisplayPort",
        Hdmi1 => "HDMI-1",
        Hdmi2 => "HDMI-2",
        UsbC => "USB-C",
        Vga => "VGA",
        Dvi => "DVI",
        _ => $"Unknown (0x{code:X2})"
    };

    public static byte GetVcpCode(InputSwitchProtocol protocol) => protocol switch
    {
        InputSwitchProtocol.Lg => 0xF4,
        _ => VcpCode
    };

    public static byte GetProtocolValue(InputSwitchProtocol protocol, byte logicalSource) => protocol switch
    {
        InputSwitchProtocol.Lg => logicalSource switch
        {
            DisplayPort => 0xD0,
            UsbC => 0xD1,
            Hdmi1 => 0x90,
            Hdmi2 => 0x90,
            _ => logicalSource
        },
        _ => logicalSource
    };
}
