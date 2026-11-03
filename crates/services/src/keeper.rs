//! The keeper bot: the permissionless steps that move money once verdicts are final, on every open
//! program (or the ones given), in a loop (`antibody keeper` once per pass: allocate, pay, pause
//! tickets, debts, re-runs, pending permits). Each pass is recorded in `keeper.json`.

use std::path::PathBuf;

use antibody_cli::{cmd, ctx::Ctx};
use eyre::Result;
use serde_json::{json, Value};

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
        let pass = json!({"at": now_s(), "programs": out});
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
                }
                Err(e) => eprintln!("keeper: {e:#}"),
            }
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
    }
}
