//! The keeper bot: the permissionless steps that move money once verdicts are final, on every open
//! program (or the ones given), in a loop (`antibody keeper` once per pass: allocate, pay, pause
//! tickets, debts, re-runs, pending permits). Each pass is recorded in `keeper.json`.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use antibody_cli::{cmd, ctx::Ctx};
use eyre::{eyre, Result};
use serde_json::{json, Value};

/// How often the keeper compares OutLayer's approved worker images with near-antibody's list.
const IMAGE_CHECK_SECS: u64 = 600;
static LAST_IMAGE_CHECK: AtomicU64 = AtomicU64::new(0);

/// OutLayer's approved images that near-antibody does not list yet, for its default registry: a run on
/// one of them gets no T1 until it is listed and the program's image delay has passed.
pub fn unlisted_images(ctx: &Ctx) -> Result<(String, usize)> {
    let defaults = ctx.view("get_defaults", json!({}))?;
    let registry = defaults["image_registry"].as_str().ok_or_else(|| eyre!("no image registry in the defaults"))?.to_string();
    let approved = ctx.near.view(&registry, "get_approved_measurements", json!({}))?;
    let known = ctx.view("get_images", json!({}))?;
    let listed: Vec<&Value> = known[0].as_array().into_iter().flatten().filter(|k| k["registry"] == json!(registry)).map(|k| &k["m"]).collect();
    let new = approved.as_array().into_iter().flatten().filter(|m| !listed.contains(m)).count();
    Ok((registry, new))
}

pub struct Keeper {
    pub ctx: Ctx,
    /// Programs to keep; empty: every open program.
    pub programs: Vec<u32>,
    pub state_file: PathBuf,
}

fn now_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Keeper {
    fn open_programs(&self) -> Result<Vec<u32>> {
        let n = self.ctx.view("get_stats", json!({}))?["next_program"].as_u64().unwrap_or(0) as u32;
        let mut out = Vec::new();
        for id in 0..n {
            if self.ctx.view("get_program", json!({"program_id": id}))?["phase"] == json!("Open") {
                out.push(id);
            }
        }
        Ok(out)
    }

    pub fn tick(&self) -> Result<Value> {
        let ids = if self.programs.is_empty() { self.open_programs()? } else { self.programs.clone() };
        let mut out = Vec::new();
        for id in ids {
            out.push(match cmd::keeper(&self.ctx, id) {
                Ok(v) => json!({"program": id, "steps": v["steps"]}),
                Err(e) => json!({"program": id, "error": format!("{e:#}")}),
            });
        }
        // OutLayer rolls out new worker images: list them at once (`sync_images`, permissionless; the
        // keeper's credit pays their storage), since each counts only after the program's image delay.
        let mut images = Value::Null;
        if now_s() >= LAST_IMAGE_CHECK.load(Ordering::Relaxed) + IMAGE_CHECK_SECS {
            LAST_IMAGE_CHECK.store(now_s(), Ordering::Relaxed);
            images = match unlisted_images(&self.ctx) {
                Ok((_, 0)) => json!({"unlisted": 0}),
                Ok((registry, n)) => match cmd::call(&self.ctx, "sync_images", json!({}), 300) {
                    Ok(s) => json!({"unlisted": n, "registry": registry, "sync_images": s["tx"], "synced": cmd::event(&s, "images_synced")}),
                    Err(e) => json!({"unlisted": n, "registry": registry, "error": format!("{e:#}")}),
                },
                Err(e) => json!({"error": format!("{e:#}")}),
            };
        }
        let pass = json!({"at": now_s(), "programs": out, "images": images});
        if let Some(d) = self.state_file.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(&self.state_file, serde_json::to_vec_pretty(&pass)?)?;
        Ok(pass)
    }

    pub fn run(&self, interval_ms: u64) -> Result<()> {
        loop {
            match self.tick() {
                Ok(p) => {
                    let steps: usize = p["programs"].as_array().into_iter().flatten().map(|x| x["steps"].as_array().map(|s| s.len()).unwrap_or(0)).sum();
                    if steps > 0 {
                        println!("{}", json!({"keeper": p["at"], "steps": steps}));
                    }
                    if p["images"].get("sync_images").is_some() || p["images"].get("error").is_some() {
                        println!("{}", json!({"keeper": p["at"], "images": p["images"]}));
                    }
                }
                Err(e) => eprintln!("keeper: {e:#}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}
