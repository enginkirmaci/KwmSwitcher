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

    public const byte PipVcpCode = 0xCC;
    public const byte PipOff = 0x00;
    public const byte PipOn = 0x01;
    public const byte PipPbp = 0x02;

    public const byte LgInputVcpCode = 0xF4;
    public const byte LgPipVcpCode = 0xD7;
    public const byte LgPipOff = 0x01;
    public const byte LgPipPbp = 0x05;

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

    public static string GetPipModeName(byte code) => code switch
    {
        PipOff => "Off",
        PipOn => "PiP",
        PipPbp => "PBP",
        _ => $"Unknown (0x{code:X2})"
    };

    public static bool IsPipActive(byte code) => code != PipOff;

    public static byte GetVcpCode(InputSwitchProtocol protocol) => protocol switch
    {
        InputSwitchProtocol.Lg => LgInputVcpCode,
        _ => VcpCode
    };

    public static byte GetPipVcpCode(InputSwitchProtocol protocol) => protocol switch
    {
        InputSwitchProtocol.Lg => LgPipVcpCode,
        _ => PipVcpCode
    };

    public static byte GetInputI2cSourceAddress(InputSwitchProtocol protocol) => protocol switch
    {
        InputSwitchProtocol.Lg => 0x50,
        _ => 0x00
    };

    public static byte GetPipI2cSourceAddress(InputSwitchProtocol protocol) => protocol switch
    {
        InputSwitchProtocol.Lg => 0x51,
        _ => 0x00
    };

    public static byte GetProtocolValue(InputSwitchProtocol protocol, byte logicalSource) => protocol switch
    {
        InputSwitchProtocol.Lg => logicalSource switch
        {
            DisplayPort => 0xD0,
            UsbC => 0xD1,
            Hdmi1 => 0x90,
            Hdmi2 => 0x91,
            _ => logicalSource
        },
        _ => logicalSource
    };

    public static byte DecodeInputSource(InputSwitchProtocol protocol, byte wireValue) => protocol switch
    {
        InputSwitchProtocol.Lg => wireValue switch
        {
            0xD0 => DisplayPort,
            0xD1 => UsbC,
            0x90 => Hdmi1,
            0x91 => Hdmi2,
            _ => wireValue
        },
        _ => wireValue
    };

    public static byte GetPipProtocolValue(InputSwitchProtocol protocol, byte mode) => protocol switch
    {
        InputSwitchProtocol.Lg => mode switch
        {
            PipOff => LgPipOff,
            PipOn => LgPipPbp,
            PipPbp => LgPipPbp,
            _ => mode
        },
        _ => mode
    };

    public static byte DecodePipMode(InputSwitchProtocol protocol, byte wireValue) => protocol switch
    {
        InputSwitchProtocol.Lg => wireValue switch
        {
            LgPipOff => PipOff,
            LgPipPbp => PipPbp,
            _ => wireValue
        },
        _ => wireValue
    };

    public static byte CanonicalizePipMode(InputSwitchProtocol protocol, byte mode)
        => DecodePipMode(protocol, GetPipProtocolValue(protocol, mode));
}
