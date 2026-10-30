<#
.SYNOPSIS
Compress and decompress data with the Windows Compression API (cabinet.dll).

.DESCRIPTION
The tests do not use this script. Use it on Windows to make the test fixtures in
tests/fixtures/, and to compare the crate with the Windows implementation.

Use -Raw for a raw LZMS block, as WIM and ESD files store it. Without -Raw, the API writes
the buffer format, with its header and its block headers.

.EXAMPLE
./scripts/lzms-oracle.ps1 compress -In data.bin -Out data.lzms -Raw
./scripts/lzms-oracle.ps1 decompress -In data.lzms -Out data.out -Raw -Size 131072
./scripts/lzms-oracle.ps1 compress -In data.bin -Out data.buf -BlockSize 1048576
./scripts/lzms-oracle.ps1 decompress -In data.buf -Out data.out
#>
param(
    [Parameter(Mandatory, Position = 0)][ValidateSet('compress', 'decompress')][string]$Command,
    [Parameter(Mandatory)][string]$In,
    [Parameter(Mandatory)][string]$Out,
    # COMPRESS_ALGORITHM_MSZIP = 2, XPRESS = 3, XPRESS_HUFF = 4, LZMS = 5.
    [UInt32]$Algorithm = 5,
    # Set COMPRESS_RAW (0x20000000): no buffer header and no block headers.
    [switch]$Raw,
    # The uncompressed size. A raw block needs it. Zero uses a size from the buffer header.
    [UInt64]$Size = 0,
    # COMPRESS_INFORMATION_CLASS_BLOCK_SIZE. Zero keeps the default.
    [UInt32]$BlockSize = 0
)

$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class CompressionApi {
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool CreateCompressor(uint Algorithm, IntPtr AllocationRoutines, out IntPtr Handle);
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool SetCompressorInformation(IntPtr Handle, int InformationClass, ref uint Information, UIntPtr Size);
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool Compress(IntPtr Handle, byte[] Data, UIntPtr Size, byte[] Buffer, UIntPtr BufferSize, out UIntPtr DataSize);
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool CloseCompressor(IntPtr Handle);
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool CreateDecompressor(uint Algorithm, IntPtr AllocationRoutines, out IntPtr Handle);
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool Decompress(IntPtr Handle, byte[] Data, UIntPtr Size, byte[] Buffer, UIntPtr BufferSize, out UIntPtr DataSize);
    [DllImport("cabinet.dll", SetLastError = true)]
    static extern bool CloseDecompressor(IntPtr Handle);

    static Exception Fail() { return new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error()); }

    public static byte[] Pack(byte[] data, uint algorithm, uint blockSize) {
        IntPtr h;
        if (!CreateCompressor(algorithm, IntPtr.Zero, out h)) throw Fail();
        try {
            // COMPRESS_INFORMATION_CLASS_BLOCK_SIZE = 1.
            if (blockSize != 0 && !SetCompressorInformation(h, 1, ref blockSize, (UIntPtr)4)) throw Fail();
            var buffer = new byte[data.Length * 2L + 65536];
            UIntPtr size;
            if (!Compress(h, data, (UIntPtr)data.Length, buffer, (UIntPtr)buffer.Length, out size)) throw Fail();
            Array.Resize(ref buffer, (int)size);
            return buffer;
        } finally { CloseCompressor(h); }
    }

    public static byte[] Unpack(byte[] data, uint algorithm, ulong size) {
        IntPtr h;
        if (!CreateDecompressor(algorithm, IntPtr.Zero, out h)) throw Fail();
        try {
            UIntPtr needed;
            if (size == 0) {
                // Get the size from the buffer header: a call with no buffer fails and gives it.
                Decompress(h, data, (UIntPtr)data.Length, null, UIntPtr.Zero, out needed);
                size = (ulong)needed;
            }
            var buffer = new byte[size];
            if (!Decompress(h, data, (UIntPtr)data.Length, buffer, (UIntPtr)buffer.Length, out needed)) throw Fail();
            Array.Resize(ref buffer, (int)needed);
            return buffer;
        } finally { CloseDecompressor(h); }
    }
}
'@

$algorithmId = $Algorithm
if ($Raw) { $algorithmId = $algorithmId -bor 0x20000000 }
$inPath = (Resolve-Path $In).Path
$outPath = [System.IO.Path]::GetFullPath($Out, (Get-Location).Path)
$bytes = [System.IO.File]::ReadAllBytes($inPath)

switch ($Command) {
    'compress' { $result = [CompressionApi]::Pack($bytes, $algorithmId, $BlockSize) }
    'decompress' { $result = [CompressionApi]::Unpack($bytes, $algorithmId, $Size) }
}
[System.IO.File]::WriteAllBytes($outPath, $result)
"{0}: {1} bytes" -f $Out, $result.Length
