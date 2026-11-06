def main [] {
  let choice =  ["ALL", "compnode-revamped"] | input list "what to zip"
  match $choice {
    "ALL" => {
        let item = [
          "crates",
          "Cargo.toml"
        ]
          ouch compress  -y ...$item ("rembg-" + (date now | format date "%Y-%m-%d %H-%M") + ".zip")
    }
    "compnode-revamped" => {
      let item = [
        "crates/compnode-core",
        "crates/compnode-editor",
        "crates/compnode-image",
        "crates/compnode-ui",
      ]

      ouch compress  -y ...$item ("compnode-revamped-" + (date now | format date "%Y-%m-%d %H-%M") + ".zip")
    }
    _ => { print "Nothing to see here" }
    }

}