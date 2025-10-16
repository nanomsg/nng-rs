#!/usr/bin/env pwsh

# Check if Rust already installed
$have_rustup = $false
try {
    if (Get-Command rustup) {
        $have_rustup = $true
    }
} catch {
    # Do nothing
}

if ($have_rustup) {
    rustup update
} else {
    # Install Rust
    if ($IsWindows) {
        Invoke-WebRequest https://win.rustup.rs/ -OutFile rustup-init.exe
        ./rustup-init.exe -yv --default-toolchain stable --default-host x86_64-pc-windows-msvc
    } elseif ($IsMacOS) {
        Invoke-WebRequest https://sh.rustup.rs -OutFile rustup-init.sh
        bash rustup-init.sh -y -v --default-toolchain stable
    } else {
    
    }
}

# Install required version of cmake, if necessary
$cmake_version = [System.Version]"0.0"
try {
    # Parse `cmake version X.Y.Z` into version
    $cmake_version = cmake --version | Select -First 1 | %{ [System.Version]$_.Split()[2] }
} catch {
    # Do nothing
}

if ($cmake_version -lt [System.Version]"3.13") {
    if ($IsWindows) {

    } elseif ($IsMacOS) {

    } else {
        # Remove any existing old versions
        sudo apt remove -y cmake cmake-data
        # Download and install recent cmake
        Invoke-WebRequest https://github.com/Kitware/CMake/releases/download/v3.16.4/cmake-3.16.4-Linux-x86_64.sh -OutFile cmake.sh
        sudo sh cmake.sh --skip-license --prefix=/usr/local
    }
}
