let choice = ["install qr", "install plugin"] | input list "Pick action: "
let plugin_out = "C:/Users/jmkl/AppData/Roaming/Adobe/UXP/Plugins/External/hello.dcsms.quickrestore_1.0.0/"
let binout = "C:/bin/"

def main [] {

match $choice {
  "install qr"      => {
    print "installing qr..."
    let root = $env.PWD | path dirname | path dirname  | path join "target" "release"
    let dlls = ls $root | where name =~ "dll" | get name
    let qr = $root | path join "qr.exe"
    for x in $dlls {
      if ($x | path exists) {
        print $"copying ($x)"
        cp $x $binout
      }
    }
    if ($qr | path exists) {
        print $"copying ($qr)"
      cp $qr $binout
    }
  }
  "install plugin" => {

    print "installing plugin..."
      ouch decompress hello.dcsms.quickrestore_PS.ccx --dir $plugin_out -y
  }
  _                 => {

    print "bye..." }
  }

}