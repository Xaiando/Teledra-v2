//! Device-free evidence for the court's measured preview acceptance floor.
use std::path::PathBuf;
use teledra_fractus::{Scene, VisualRichness, write_manifest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let output = PathBuf::from(
        args.next()
            .unwrap_or_else(|| "target/fractus-richness".into()),
    );
    std::fs::create_dir_all(&output)?;
    if let Some(original) = args.next() {
        let image = image::open(&original)?.to_rgb8();
        let facts = VisualRichness::from_image(&image);
        println!(
            "{}",
            serde_json::json!({"original":original,"facts":facts,"concerns":facts.concerns()})
        );
    }
    for family in [
        "rose_curve",
        "lotus_mandala",
        "julia",
        "kaleidoscope",
        "mandelbrot",
        "star_mandala",
        "flower_of_life",
        "newton",
        "spirograph",
        "phyllotaxis",
        "radial_weave",
        "lissajous",
        "harmonograph",
    ] {
        let dsl = format!(
            "version 2\ntype={family}\nwidth=600\nheight=800\niterations=180\nseed=11036126381597215977\npalette=solar_gold\nsymmetry=12\nzoom=1\nmotion=0.65"
        );
        let scene = Scene::parse(&dsl)?;
        let manifest = scene.render(&output.join(format!("{family}.png")))?;
        write_manifest(&output.join(format!("{family}.manifest.json")), &manifest)?;
        let facts = manifest.visual_richness.as_ref().unwrap();
        println!(
            "{}",
            serde_json::json!({"family":family,"facts":facts,"concerns":facts.concerns()})
        );
    }
    let scene = Scene::parse(
        "type=flower_of_life; width=600; height=800; iterations=180; seed=3187; palette=emerald; symmetry=6; zoom=1; motion=0.6",
    )?;
    let manifest = scene.render(&output.join("curated-flower-of-life.png"))?;
    write_manifest(
        &output.join("curated-flower-of-life.manifest.json"),
        &manifest,
    )?;
    let facts = scene.require_visual_richness()?;
    println!(
        "{}",
        serde_json::json!({"family":"curated-flower-of-life","facts":facts,"concerns":facts.concerns()})
    );
    Ok(())
}
