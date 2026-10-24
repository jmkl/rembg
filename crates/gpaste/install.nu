
# install.nu

$env.config = ($env.config | upsert shell_integration osc)

let root = ($env.PSScriptRoot? | default (pwd | path expand))
let root = ($root | path dirname)
let rembg_root = ($root | path dirname)

let host_name = "hello.dcsms.gpaste"
let install_dir = ($env.LOCALAPPDATA | path join "GPasTe")

# Create installation directory
mkdir $install_dir

# Build the Rust native host
do {
    cd $root
    cargo build --release -p gpaste
}

if $env.LAST_EXIT_CODE != 0 {
    error make {msg: "cargo build failed"}
}

# Copy the executable
let source_exe = ($rembg_root | path join "target" "release" "gpaste_host.exe")
let target_exe = ($install_dir | path join "gpaste_host.exe")

if not ($source_exe | path exists) {
    error make {msg: $"Build output not found: ($source_exe)"}
}

cp --force $source_exe $target_exe

# Ask for the extension ID
let extension_id = (input "Extension Id: ")

if ($extension_id | str trim | is-empty) {
    error make {msg: "Extension ID is required."}
}

# Generate the native messaging manifest
let manifest_path = ($install_dir | path join "hello.dcsms.gpaste.json")

let manifest = {
    name: $host_name
    description: "Rust GUI to ChatGPT bridge"
    path: $target_exe
    type: "stdio"
    allowed_origins: [
        $"chrome-extension://($extension_id)/"
    ]
}

$manifest
    | to json --indent 2
    | save --force --raw $manifest_path

# Register the native messaging host for Chrome and Vivaldi
let registry_paths = [
    "HKCU\\Software\\Google\\Chrome\\NativeMessagingHosts\\hello.dcsms.gpaste"
    "HKCU\\Software\\Vivaldi\\NativeMessagingHosts\\hello.dcsms.gpaste"
]

for registry_path in $registry_paths {
    # Create the registry key
    ^reg add $registry_path /f

    if $env.LAST_EXIT_CODE != 0 {
        error make {msg: $"Failed to create registry key: ($registry_path)"}
    }

    # Set the default value to the manifest path
    ^reg add $registry_path /ve /t REG_SZ /d $manifest_path /f

    if $env.LAST_EXIT_CODE != 0 {
        error make {msg: $"Failed to register native host: ($registry_path)"}
    }
}

print $"Installed native host: ($manifest_path)"
print "Load the extension folder as Unpacked in chrome://extensions, then restart Chrome."
