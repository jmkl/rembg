def main [] {
  let item = [
    "crates/compnode",
    "crates/gpaste",
    "crates/rembg-core",
    "Cargo.toml"
  ]
    ouch compress  -y ...$item ("compnode-old-" + (date now | format date "%Y-%m-%d %H-%M") + ".zip")

}