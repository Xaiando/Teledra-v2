//! Render actual production scenes for visual review, without the live court.
use std::path::PathBuf;
use std::time::Instant;
use teledra_fractus::{Scene, write_manifest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let output = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/fractus-overhaul/gallery"));
    let animated = args.iter().any(|arg| arg == "--animate");
    let selected = args
        .windows(2)
        .find(|pair| pair[0] == "--only")
        .map(|pair| pair[1].as_str());
    std::fs::create_dir_all(&output)?;
    let recipes = [
        (
            "01-lotus-observatory",
            "lotus_mandala",
            "electric_cyan",
            731,
            12,
            1.0,
        ),
        ("02-ember-julia", "julia", "ember", 219, 12, 1.1),
        (
            "03-astral-star",
            "star_mandala",
            "purple_haze",
            1087,
            18,
            1.0,
        ),
        (
            "04-living-lattice",
            "flower_of_life",
            "solar_gold",
            463,
            6,
            1.0,
        ),
        (
            "05-recursive-garden",
            "kaleidoscope",
            "neon_sunset",
            1771,
            10,
            1.0,
        ),
        ("06-copper-seahorse", "mandelbrot", "ice_fire", 129, 12, 1.7),
        ("07-orbit-weave", "radial_weave", "emerald", 83, 16, 1.0),
        (
            "08-seed-constellation",
            "phyllotaxis",
            "twilight",
            639,
            12,
            1.0,
        ),
        (
            "09-luminous-roulette",
            "spirograph",
            "electric_cyan",
            463,
            12,
            1.0,
        ),
    ];
    for (title, kind, palette, seed, symmetry, zoom) in recipes {
        if selected.is_some_and(|selected| !title.contains(selected) && kind != selected) {
            continue;
        }
        let dsl = format!(
            "version 2\ntype={kind}\nwidth=480\nheight=600\niterations=220\nseed={seed}\npalette={palette}\nsymmetry={symmetry}\nzoom={zoom}\nmotion=0.65"
        );
        std::fs::write(output.join(format!("{title}.fractus")), &dsl)?;
        let scene = Scene::parse(&dsl)?;
        let start = Instant::now();
        let manifest = if animated {
            scene.render_animation(&output.join(format!("{title}.png")), 120, 24)?
        } else {
            scene.render(&output.join(format!("{title}-still.png")))?
        };
        write_manifest(&output.join(format!("{title}.manifest.json")), &manifest)?;
        println!(
            "{}",
            serde_json::json!({"scene":title,"seconds":start.elapsed().as_secs_f64(),"manifest":manifest})
        );
    }
    Ok(())
}
