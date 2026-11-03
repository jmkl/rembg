def main [] {
  let item = [
    "crates",
    "Cargo.toml"
  ]
    ouch compress  -y ...$item ("rembg-" + (date now | format date "%Y-%m-%d %H-%M") + ".zip")

}