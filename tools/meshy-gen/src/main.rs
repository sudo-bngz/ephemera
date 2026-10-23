//! meshy-gen — generates Ephemera's models with the Meshy text-to-3D API.
//!
//! ```sh
//! export MESHY_API_KEY=msy_...
//! cargo run --release --manifest-path tools/meshy-gen/Cargo.toml            # every missing model
//! cargo run --release --manifest-path tools/meshy-gen/Cargo.toml -- ephemera gate
//! cargo run --release --manifest-path tools/meshy-gen/Cargo.toml -- --dry-run
//! ```
//!
//! For each model: a *preview* task (geometry, remeshed to the poly budget),
//! then a *refine* task (textures), then the GLB is downloaded to
//! `assets/models/<name>.glb`, where the game picks it up on the next run.
//!
//! Every task id is recorded in `assets/models/.meshy-tasks`, so an
//! interrupted run resumes instead of paying for the same model twice.
//! Models run in parallel; `--force` regenerates models that already exist.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const DEFAULT_BASE: &str = "https://api.meshy.ai";

#[derive(Debug, Clone, PartialEq)]
struct Spec {
    name: String,
    prompt: String,
    texture: Option<String>,
    polycount: u32,
    pose: Option<String>,
}

/// Parses `meshy-prompts.txt`: `[name]` sections with `key = value` lines.
/// A value may continue on following indented lines.
fn parse_prompts(text: &str) -> Result<Vec<Spec>, String> {
    let mut out: Vec<Spec> = Vec::new();
    let mut last_key: Option<String> = None;
    for (n, raw) in text.lines().enumerate() {
        let line_no = n + 1;
        if raw.trim_start().starts_with('#') || raw.trim().is_empty() {
            continue;
        }
        let line = raw.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            out.push(Spec {
                name: name.trim().to_string(),
                prompt: String::new(),
                texture: None,
                polycount: 20_000,
                pose: None,
            });
            last_key = None;
            continue;
        }
        let Some(spec) = out.last_mut() else {
            return Err(format!("line {line_no}: setting outside of a [model] section"));
        };
        // Continuation of the previous value.
        if raw.starts_with(char::is_whitespace) && !line.contains(" = ") {
            match last_key.as_deref() {
                Some("prompt") => {
                    spec.prompt.push(' ');
                    spec.prompt.push_str(line);
                }
                Some("texture") => {
                    let t = spec.texture.get_or_insert_with(String::new);
                    t.push(' ');
                    t.push_str(line);
                }
                _ => return Err(format!("line {line_no}: unexpected continuation")),
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("line {line_no}: expected `key = value`"));
        };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "prompt" => spec.prompt = v.to_string(),
            "texture" => spec.texture = Some(v.to_string()),
            "polycount" => {
                spec.polycount = v
                    .parse()
                    .map_err(|_| format!("line {line_no}: polycount must be a number"))?
            }
            "pose" => spec.pose = Some(v.to_string()).filter(|p| !p.is_empty()),
            _ => return Err(format!("line {line_no}: unknown key `{k}`")),
        }
        last_key = Some(k.to_string());
    }
    for s in &out {
        if s.prompt.is_empty() {
            return Err(format!("[{}] has no prompt", s.name));
        }
        if s.prompt.len() > 800 {
            return Err(format!("[{}] prompt is over Meshy's 800 characters", s.name));
        }
        if !(100..=300_000).contains(&s.polycount) {
            return Err(format!("[{}] polycount must be 100–300000", s.name));
        }
    }
    Ok(out)
}

/// `name stage task_id` lines, so runs can resume.
#[derive(Default)]
struct Ledger {
    path: PathBuf,
    entries: BTreeMap<(String, String), String>,
}

impl Ledger {
    fn load(path: PathBuf) -> Self {
        let mut entries = BTreeMap::new();
        if let Ok(text) = fs::read_to_string(&path) {
            for line in text.lines() {
                let mut p = line.split_whitespace();
                if let (Some(n), Some(s), Some(id)) = (p.next(), p.next(), p.next()) {
                    entries.insert((n.to_string(), s.to_string()), id.to_string());
                }
            }
        }
        Self { path, entries }
    }

    fn get(&self, name: &str, stage: &str) -> Option<String> {
        self.entries.get(&(name.into(), stage.into())).cloned()
    }

    fn set(&mut self, name: &str, stage: &str, id: &str) {
        self.entries.insert((name.into(), stage.into()), id.into());
        self.save();
    }

    fn forget(&mut self, name: &str, stage: &str) {
        self.entries.remove(&(name.to_string(), stage.to_string()));
        self.save();
    }

    fn clear(&mut self, name: &str) {
        self.entries.retain(|(n, _), _| n != name);
        self.save();
    }

    fn save(&self) {
        let text: String = self
            .entries
            .iter()
            .map(|((n, s), id)| format!("{n} {s} {id}\n"))
            .collect();
        let _ = fs::write(&self.path, text);
    }
}

struct Api {
    base: String,
    key: String,
    agent: ureq::Agent,
}

impl Api {
    fn create(&self, body: Value) -> Result<String, String> {
        let resp = self
            .agent
            .post(&format!("{}/openapi/v2/text-to-3d", self.base))
            .set("Authorization", &format!("Bearer {}", self.key))
            .send_json(body)
            .map_err(describe)?;
        let v: Value = resp.into_json().map_err(|e| e.to_string())?;
        v["result"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| format!("no task id in response: {v}"))
    }

    fn task(&self, id: &str) -> Result<Value, String> {
        self.agent
            .get(&format!("{}/openapi/v2/text-to-3d/{id}", self.base))
            .set("Authorization", &format!("Bearer {}", self.key))
            .call()
            .map_err(describe)?
            .into_json()
            .map_err(|e| e.to_string())
    }

    /// Polls until the task finishes. Returns the final task JSON.
    /// `Err(Wait::Dead)` means Meshy failed the task: it must be recreated,
    /// not resumed. `Err(Wait::Io)` is a network problem: resume later.
    fn wait(&self, name: &str, stage: &str, id: &str, poll: Duration) -> Result<Value, Wait> {
        let mut last = -1;
        loop {
            let t = self.task(id).map_err(Wait::Io)?;
            let status = t["status"].as_str().unwrap_or("?");
            let progress = t["progress"].as_i64().unwrap_or(0);
            if progress != last {
                println!("  {name:<10} {stage:<8} {status:<12} {progress:>3}%");
                last = progress;
            }
            match status {
                "SUCCEEDED" => return Ok(t),
                "FAILED" | "CANCELED" => {
                    let msg = t["task_error"]["message"].as_str().unwrap_or("no message");
                    return Err(Wait::Dead(format!("{stage} {status}: {msg}")));
                }
                _ => thread::sleep(poll),
            }
        }
    }

    fn download(&self, url: &str, to: &Path) -> Result<u64, String> {
        // Asset URLs are pre-signed: no auth header.
        let resp = self.agent.get(url).call().map_err(describe)?;
        let mut bytes = Vec::new();
        resp.into_reader()
            .take(512 * 1024 * 1024)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        let tmp = to.with_extension("glb.part");
        fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        fs::rename(&tmp, to).map_err(|e| e.to_string())?;
        Ok(bytes.len() as u64)
    }
}

enum Wait {
    Dead(String),
    Io(String),
}

/// Waits on a task; if Meshy failed it, forgets it so the next run starts
/// that stage over instead of resuming a dead task forever.
fn wait_or_forget(
    api: &Api,
    ledger: &Mutex<Ledger>,
    name: &str,
    stage: &str,
    id: &str,
    poll: Duration,
) -> Result<Value, String> {
    api.wait(name, stage, id, poll).map_err(|e| match e {
        Wait::Dead(msg) => {
            let mut l = ledger.lock().unwrap();
            l.forget(name, stage);
            if stage == "preview" {
                // A refine of a dead preview can't exist either.
                l.forget(name, "refine");
            }
            msg
        }
        Wait::Io(msg) => msg,
    })
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, r) => {
            let body = r.into_string().unwrap_or_default();
            let hint = match code {
                401 => " — check MESHY_API_KEY",
                402 => " — not enough credits",
                429 => " — rate limited, try fewer models at once",
                _ => "",
            };
            format!("HTTP {code}{hint}: {body}")
        }
        other => other.to_string(),
    }
}

fn generate(
    api: &Api,
    ledger: &Mutex<Ledger>,
    spec: &Spec,
    out_dir: &Path,
    ai_model: &str,
    poll: Duration,
) -> Result<(), String> {
    let name = spec.name.as_str();

    // Preview: geometry.
    // Look up first, in its own statement: a guard in a `match` scrutinee
    // lives until the end of the match, and the arms lock the ledger again.
    let known_preview = ledger.lock().unwrap().get(name, "preview");
    let preview = match known_preview {
        Some(id) => {
            println!("  {name:<10} resuming preview {id}");
            id
        }
        None => {
            let mut body = json!({
                "mode": "preview",
                "prompt": spec.prompt,
                "ai_model": ai_model,
                "should_remesh": true,
                "topology": "triangle",
                "target_polycount": spec.polycount,
                "target_formats": ["glb"],
                "origin_at": "bottom",
            });
            if let Some(pose) = &spec.pose {
                body["pose_mode"] = json!(pose);
            }
            let id = api.create(body)?;
            ledger.lock().unwrap().set(name, "preview", &id);
            id
        }
    };
    wait_or_forget(api, ledger, name, "preview", &preview, poll)?;

    // Refine: textures.
    let known_refine = ledger.lock().unwrap().get(name, "refine");
    let refine = match known_refine {
        Some(id) => id,
        None => {
            let mut body = json!({
                "mode": "refine",
                "preview_task_id": preview,
                "enable_pbr": true,
                "texture_resolution": "2k",
                "target_formats": ["glb"],
            });
            if let Some(t) = &spec.texture {
                body["texture_prompt"] = json!(t);
            }
            let id = api.create(body)?;
            ledger.lock().unwrap().set(name, "refine", &id);
            id
        }
    };
    let done = wait_or_forget(api, ledger, name, "refine", &refine, poll)?;

    let url = done["model_urls"]["glb"]
        .as_str()
        .ok_or("refine succeeded but returned no GLB url")?;
    let path = out_dir.join(format!("{name}.glb"));
    let bytes = api.download(url, &path)?;
    let credits = done["consumed_credits"].as_i64().unwrap_or(0);
    println!(
        "  {name:<10} ✓ {} ({:.1} MB, {credits} credits for refine)",
        path.display(),
        bytes as f64 / 1e6
    );
    ledger.lock().unwrap().clear(name);
    Ok(())
}

fn main() {
    let mut names = Vec::new();
    let mut dry_run = false;
    let mut force = false;
    let mut base = std::env::var("MESHY_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE.into());
    let mut ai_model = "latest".to_string();
    let mut poll = Duration::from_secs(5);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dry-run" | "-n" => dry_run = true,
            "--force" | "-f" => force = true,
            "--base-url" => base = args.next().expect("--base-url needs a value"),
            "--ai-model" => ai_model = args.next().expect("--ai-model needs a value"),
            "--poll-ms" => {
                poll = Duration::from_millis(
                    args.next().and_then(|v| v.parse().ok()).expect("--poll-ms needs a number"),
                )
            }
            "-h" | "--help" => {
                println!(
                    "usage: meshy-gen [model…] [--dry-run] [--force] [--ai-model NAME] [--base-url URL]\n\
                     Reads assets/models/meshy-prompts.txt, writes assets/models/<model>.glb.\n\
                     Needs MESHY_API_KEY."
                );
                return;
            }
            n if n.starts_with('-') => {
                eprintln!("unknown option {n}");
                std::process::exit(2);
            }
            n => names.push(n.to_string()),
        }
    }

    // Run from anywhere inside the repo: find assets/models upwards.
    let out_dir = std::env::current_dir()
        .unwrap()
        .ancestors()
        .map(|p| p.join("assets/models"))
        .find(|p| p.join("meshy-prompts.txt").exists())
        .unwrap_or_else(|| {
            eprintln!("can't find assets/models/meshy-prompts.txt — run inside the ephemera repo");
            std::process::exit(2);
        });

    let text = fs::read_to_string(out_dir.join("meshy-prompts.txt")).unwrap();
    let specs = parse_prompts(&text).unwrap_or_else(|e| {
        eprintln!("meshy-prompts.txt: {e}");
        std::process::exit(2);
    });
    for n in &names {
        if !specs.iter().any(|s| &s.name == n) {
            eprintln!("no [{n}] section in meshy-prompts.txt");
            std::process::exit(2);
        }
    }
    let todo: Vec<Spec> = specs
        .into_iter()
        .filter(|s| names.is_empty() || names.contains(&s.name))
        .filter(|s| {
            let exists = out_dir.join(format!("{}.glb", s.name)).exists();
            if exists && !force {
                println!("  {:<10} exists, skipping (--force to regenerate)", s.name);
            }
            force || !exists
        })
        .collect();

    if todo.is_empty() {
        println!("nothing to generate");
        return;
    }
    println!(
        "{} model(s) via {base} (ai_model {ai_model}): {}",
        todo.len(),
        todo.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ")
    );
    if dry_run {
        for s in &todo {
            println!("\n[{}] {} tris, pose {:?}\n  {}", s.name, s.polycount, s.pose, s.prompt);
            if let Some(t) = &s.texture {
                println!("  texture: {t}");
            }
        }
        return;
    }

    let key = std::env::var("MESHY_API_KEY").unwrap_or_else(|_| {
        eprintln!("set MESHY_API_KEY (https://www.meshy.ai/settings/api)");
        std::process::exit(2);
    });
    let api = Arc::new(Api {
        base: base.trim_end_matches('/').to_string(),
        key,
        agent: ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(120))
            .build(),
    });
    let ledger = Arc::new(Mutex::new(Ledger::load(out_dir.join(".meshy-tasks"))));

    let handles: Vec<_> = todo
        .into_iter()
        .map(|spec| {
            let (api, ledger, out_dir, ai_model) =
                (api.clone(), ledger.clone(), out_dir.clone(), ai_model.clone());
            thread::spawn(move || {
                let r = generate(&api, &ledger, &spec, &out_dir, &ai_model, poll);
                if let Err(e) = &r {
                    eprintln!("  {:<10} ✗ {e}", spec.name);
                }
                r.is_ok()
            })
        })
        .collect();
    let failed = handles
        .into_iter()
        .map(|h| h.join().unwrap_or(false))
        .filter(|ok| !ok)
        .count();
    if failed > 0 {
        eprintln!("{failed} model(s) failed — re-run to resume where they stopped");
        std::process::exit(1);
    }
    println!("done — run the game and the new models are in.");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_and_continuations() {
        let specs = parse_prompts(
            "# c\n[a]\nprompt = one\n  two\npolycount = 500\npose = t-pose\n\n[b]\nprompt = x\ntexture = y\n",
        )
        .unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].prompt, "one two");
        assert_eq!(specs[0].polycount, 500);
        assert_eq!(specs[0].pose.as_deref(), Some("t-pose"));
        assert_eq!(specs[1].texture.as_deref(), Some("y"));
        assert_eq!(specs[1].polycount, 20_000);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse_prompts("prompt = x").is_err());
        assert!(parse_prompts("[a]\npolycount = 5\nprompt = x").is_err());
        assert!(parse_prompts("[a]\nbogus = 1\nprompt = x").is_err());
        assert!(parse_prompts("[a]\npolycount = 500").is_err());
    }

    #[test]
    fn shipped_prompts_are_valid() {
        let text = include_str!("../../../assets/models/meshy-prompts.txt");
        let specs = parse_prompts(text).unwrap();
        let names: Vec<_> = specs.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["ephemera", "judith", "reed_tall", "reed", "gate", "mesa"]);
    }
}
