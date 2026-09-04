//! Runs the payload shapes seen on stream through the scene parser.

fn main() {
    let cases = [
        "layers=kaleidoscope@0.65@add@electric_purple;lissajous@0.35@multiply@electric_cyan",
        "layers=moire@3@screen@electric_cyan;rose_curve@0.3@add@violet",
        "type=mandala\niterations=220\npalette=neon_sunset",
        "palette=rose_gold",
        "palette=] To explore subtle shadow manipulation in generative art",
        "layers=reeds_and_inkwells@0.2@screen@violet",
        "layers=sierpinski@0.5@normal@twilight;reeds_and_inkwells@0.2@add@gold",
    ];
    for case in cases {
        let dsl = format!("version 2\n{case}");
        match teledra_fractus::Scene::parse(&dsl) {
            Ok(scene) => println!(
                "  OK    kind={:<14} palette={:<13} layers={} unknown={:?}",
                format!("{:?}", scene.kind),
                scene.palette,
                scene.layers.len(),
                scene.unknown_layers
            ),
            Err(error) => println!("  REFUSED  {error}"),
        }
        println!(
            "        <- {}",
            case.replace('\n', " | ")
                .chars()
                .take(76)
                .collect::<String>()
        );
    }
}
