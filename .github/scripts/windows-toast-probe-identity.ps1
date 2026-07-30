param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('Create', 'Remove')]
    [string] $Mode,

    [Parameter(Mandatory = $true)]
    [string] $ShortcutPath,

    [string] $TargetPath,

    [string] $TargetArguments = '',

    [string] $IconPath
)

$ErrorActionPreference = 'Stop'
$applicationId = 'LeopardRich.CodexNotifier'
$identityKey = "HKCU:\Software\Classes\AppUserModelId\$applicationId"

if ($Mode -eq 'Remove') {
    if (Test-Path -LiteralPath $ShortcutPath) {
        Remove-Item -LiteralPath $ShortcutPath -Force
    }
    if (Test-Path -LiteralPath $identityKey) {
        Remove-Item -LiteralPath $identityKey -Recurse -Force
    }
    return
}

if (-not $TargetPath -or -not (Test-Path -LiteralPath $TargetPath)) {
    throw 'An existing TargetPath is required in Create mode'
}
if (-not $IconPath -or -not (Test-Path -LiteralPath $IconPath)) {
    throw 'An existing IconPath is required in Create mode'
}
if ((Test-Path -LiteralPath $ShortcutPath) -or (Test-Path -LiteralPath $identityKey)) {
    throw 'Probe Toast identity already exists'
}

$source = @'
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;

public static class ProbeShortcutIdentity
{
    [ComImport]
    [Guid("00021401-0000-0000-C000-000000000046")]
    private class ShellLink { }

    [ComImport]
    [Guid("000214F9-0000-0000-C000-000000000046")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IShellLinkW
    {
        [PreserveSig] int GetPath(IntPtr path, int pathLength, IntPtr findData, uint flags);
        [PreserveSig] int GetIdList(out IntPtr itemIdList);
        [PreserveSig] int SetIdList(IntPtr itemIdList);
        [PreserveSig] int GetDescription(IntPtr description, int descriptionLength);
        [PreserveSig] int SetDescription([MarshalAs(UnmanagedType.LPWStr)] string description);
        [PreserveSig] int GetWorkingDirectory(IntPtr directory, int directoryLength);
        [PreserveSig] int SetWorkingDirectory([MarshalAs(UnmanagedType.LPWStr)] string directory);
        [PreserveSig] int GetArguments(IntPtr arguments, int argumentsLength);
        [PreserveSig] int SetArguments([MarshalAs(UnmanagedType.LPWStr)] string arguments);
        [PreserveSig] int GetHotkey(out short hotkey);
        [PreserveSig] int SetHotkey(short hotkey);
        [PreserveSig] int GetShowCommand(out int showCommand);
        [PreserveSig] int SetShowCommand(int showCommand);
        [PreserveSig] int GetIconLocation(IntPtr iconPath, int iconPathLength, out int iconIndex);
        [PreserveSig] int SetIconLocation([MarshalAs(UnmanagedType.LPWStr)] string iconPath, int iconIndex);
        [PreserveSig] int SetRelativePath([MarshalAs(UnmanagedType.LPWStr)] string path, uint reserved);
        [PreserveSig] int Resolve(IntPtr window, uint flags);
        [PreserveSig] int SetPath([MarshalAs(UnmanagedType.LPWStr)] string path);
    }

    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    private struct PropertyKey
    {
        public Guid FormatId;
        public uint PropertyId;

        public PropertyKey(Guid formatId, uint propertyId)
        {
            FormatId = formatId;
            PropertyId = propertyId;
        }
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct PropVariant
    {
        [FieldOffset(0)] public ushort VariantType;
        [FieldOffset(8)] public IntPtr PointerValue;
    }

    [ComImport]
    [Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IPropertyStore
    {
        [PreserveSig] int GetCount(out uint count);
        [PreserveSig] int GetAt(uint index, out PropertyKey key);
        [PreserveSig] int GetValue(ref PropertyKey key, out PropVariant value);
        [PreserveSig] int SetValue(ref PropertyKey key, ref PropVariant value);
        [PreserveSig] int Commit();
    }

    [DllImport("ole32.dll", PreserveSig = true)]
    private static extern int PropVariantClear(ref PropVariant value);

    [DllImport("shell32.dll")]
    private static extern void SHChangeNotify(uint eventId, uint flags, IntPtr item1, IntPtr item2);

    public static void Create(
        string shortcutPath,
        string targetPath,
        string targetArguments,
        string iconPath,
        string appId)
    {
        var shellLink = (IShellLinkW)new ShellLink();
        try
        {
            Marshal.ThrowExceptionForHR(shellLink.SetPath(targetPath));
            Marshal.ThrowExceptionForHR(shellLink.SetWorkingDirectory(
                System.IO.Path.GetDirectoryName(targetPath)));
            Marshal.ThrowExceptionForHR(shellLink.SetDescription("Codex Notifier"));
            Marshal.ThrowExceptionForHR(shellLink.SetArguments(targetArguments));
            Marshal.ThrowExceptionForHR(shellLink.SetIconLocation(iconPath, 0));

            var store = (IPropertyStore)shellLink;
            var key = new PropertyKey(
                new Guid("9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3"), 5);
            var value = new PropVariant
            {
                VariantType = 31,
                PointerValue = Marshal.StringToCoTaskMemUni(appId)
            };
            try
            {
                Marshal.ThrowExceptionForHR(store.SetValue(ref key, ref value));
                Marshal.ThrowExceptionForHR(store.Commit());
            }
            finally
            {
                PropVariantClear(ref value);
            }

            ((IPersistFile)shellLink).Save(shortcutPath, true);
        }
        finally
        {
            Marshal.FinalReleaseComObject(shellLink);
        }

        SHChangeNotify(0x08000000, 0, IntPtr.Zero, IntPtr.Zero);
    }
}
'@

try {
    New-Item -Path $identityKey -Force | Out-Null
    New-ItemProperty -Path $identityKey -Name DisplayName -Value 'Codex Notifier' -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $identityKey -Name IconBackgroundColor -Value '0' -PropertyType String -Force | Out-Null
    New-ItemProperty -Path $identityKey -Name IconUri -Value $IconPath -PropertyType String -Force | Out-Null

    Add-Type -TypeDefinition $source -Language CSharp
    [ProbeShortcutIdentity]::Create(
        $ShortcutPath,
        $TargetPath,
        $TargetArguments,
        $IconPath,
        $applicationId)
}
catch {
    if (Test-Path -LiteralPath $ShortcutPath) {
        Remove-Item -LiteralPath $ShortcutPath -Force
    }
    if (Test-Path -LiteralPath $identityKey) {
        Remove-Item -LiteralPath $identityKey -Recurse -Force
    }
    throw
}
