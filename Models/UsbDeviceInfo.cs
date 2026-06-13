namespace KwmSwitcher.Models;

public record UsbDeviceInfo(string VendorId, string ProductId, string Description)
{
    public string Key => $"{VendorId}:{ProductId}";
}
