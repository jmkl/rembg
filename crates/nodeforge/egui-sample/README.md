# nodeforge egui sample

    cd egui-sample && cargo run --release        # needs Rust 1.84+ and a desktop session

Wheel: zoom at cursor | drag canvas / middle-drag: pan | drag node: move |
drag output pin -> input pin: connect (cycles rejected) | drag a connected
input: detach and re-route | Del: delete selected node.
Toolbar: node-count slider + Regenerate (100 to 40,000 nodes), Fit.

The top-left overlay shows visible/total counts and CPU ms per frame
(engine + shape building; excludes egui tessellation and GPU time).

Verified headless (`cargo test`, real egui frames with injected pointer
events). Not yet run in a window: this was built in a sandbox without a display.
