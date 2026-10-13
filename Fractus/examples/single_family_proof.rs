//! Compare an actual layered payload with the gallery's one-family policy.
use std::path::PathBuf;
use teledra_fractus::{Scene, single_family_dsl, write_manifest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let source = PathBuf::from(args.first().ok_or("expected source DSL path")?);
    let output = PathBuf::from(args.get(1).ok_or("expected output directory")?);
    let animate = args.iter().any(|argument| argument == "--animate");
    let original = std::fs::read_to_string(source)?;
    let clean = single_family_dsl(&original)?;
    std::fs::create_dir_all(&output)?;
    for (name, dsl) in [("before", original.as_str()), ("after", clean.as_str())] {
        std::fs::write(output.join(format!("{name}.fractus")), dsl)?;
        let scene = Scene::parse(dsl)?;
        let still = scene.render(&output.join(format!("{name}-still.png")))?;
        write_manifest(&output.join(format!("{name}-still.manifest.json")), &still)?;
        if animate {
            let animated = scene.render_animation(&output.join(format!("{name}.png")), 120, 24)?;
            write_manifest(&output.join(format!("{name}.manifest.json")), &animated)?;
        }
        println!(
            "{}",
            serde_json::json!({"variant":name,"kind":scene.kind,"layers":scene.layers.len(),"seed":scene.seed,"output":output})
        );
    }
    Ok(())
}
