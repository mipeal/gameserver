//! `scripts/sync_teams_http.py` — sync teams (and logos) from the CTFd
//! website. ARGUMENTS: [-d|--delete]

use controlserver_rs::models::Team;
use image::imageops::FilterType;
use md5::{Digest, Md5};
use serde_json::Value;

const REQUEST_TIMEOUT_SECS: u64 = 5;
const TARGET_SIZE: u32 = 256;

/// `TeamLogo.store_logo_bytes` — md5 of the source data, resize+pad to
/// 256×256 RGBA PNG, store in team_logos (if not already there).
async fn store_logo_bytes(db: &sqlx::PgPool, data: &[u8]) -> Result<String, String> {
    let imghash = format!("{:x}", Md5::digest(data));
    let existing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM team_logos WHERE hash = $1")
        .bind(&imghash)
        .fetch_one(db)
        .await
        .map_err(|e| e.to_string())?;
    if existing > 0 {
        return Ok(imghash);
    }
    let img = image::load_from_memory(data).map_err(|e| format!("ValueError: {e}"))?;
    let m = img.width().max(img.height());
    let w = ((img.width() as u64 * TARGET_SIZE as u64 + m as u64 / 2) / m as u64) as u32;
    let h = ((img.height() as u64 * TARGET_SIZE as u64 + m as u64 / 2) / m as u64) as u32;
    let resized = image::imageops::resize(&img, w, h, FilterType::CatmullRom);
    let mut canvas =
        image::RgbaImage::from_pixel(TARGET_SIZE, TARGET_SIZE, image::Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut canvas,
        &resized,
        ((TARGET_SIZE - w) / 2) as i64,
        ((TARGET_SIZE - h) / 2) as i64,
    );
    let mut out = std::io::Cursor::new(Vec::new());
    canvas
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    sqlx::query("INSERT INTO team_logos (hash, content) VALUES ($1, $2)")
        .bind(&imghash)
        .bind(out.into_inner())
        .execute(db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(imghash)
}

fn logo_dirs(cfg: &saarctf_commons_rs::config::Config) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![std::path::PathBuf::from(&cfg.scoreboard_path)];
    if let Some(p) = &cfg.scoreboard_path_internal {
        dirs.push(std::path::PathBuf::from(p));
    }
    dirs
}

async fn import_logo_from_bytes(
    db: &sqlx::PgPool,
    cfg: &saarctf_commons_rs::config::Config,
    team: &mut Team,
    data: &[u8],
) {
    for d in logo_dirs(cfg) {
        let _ = std::fs::create_dir_all(d.join("logos"));
    }
    match store_logo_bytes(db, data).await {
        Ok(hash) => team.logo = Some(hash),
        Err(e) => println!("[ERROR] logo: {e}"),
    }
}

async fn import_logo(db: &sqlx::PgPool, cfg: &saarctf_commons_rs::config::Config, team: &mut Team) {
    // profile_dummy.png from the repo root, like the Python default
    let root = std::env::var("SAARCTF_PYTHON_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".."));
    let dummy = root
        .join("controlserver")
        .join("static")
        .join("img")
        .join("profile_dummy.png");
    match std::fs::read(&dummy) {
        Ok(data) => import_logo_from_bytes(db, cfg, team, &data).await,
        Err(e) => println!(
            "[ERROR] dummy logo missing ({}): {e}",
            dummy.to_string_lossy()
        ),
    }
}

async fn import_logo_from_url(
    db: &sqlx::PgPool,
    cfg: &saarctf_commons_rs::config::Config,
    team: &mut Team,
    fname: &str,
) {
    if fname.is_empty() {
        import_logo(db, cfg, team).await;
        return;
    }
    // image already present?
    if let Some(logo) = &team.logo {
        if fname.contains(logo.as_str()) {
            return;
        }
    }
    let url = match cfg.config.get("website_logo_url").and_then(Value::as_str) {
        Some(base) => format!("{base}/{fname}"),
        None => {
            let base = cfg
                .config
                .get("website_team_url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .split('?')
                .next()
                .unwrap_or_default()
                .trim_end_matches('/')
                .rsplitn(3, '/')
                .last()
                .unwrap_or_default()
                .to_string();
            format!("{base}/media/{fname}")
        }
    };
    let client = reqwest::Client::new();
    match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
    {
        Ok(resp) if resp.status() == reqwest::StatusCode::OK => match resp.bytes().await {
            Ok(data) => import_logo_from_bytes(db, cfg, team, &data).await,
            Err(e) => println!("Could not fetch logo {e}"),
        },
        Ok(resp) => println!("Could not fetch logo status={}", resp.status()),
        Err(e) => println!("Could not fetch logo {e}"),
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let should_delete = args.iter().any(|a| a == "-d" || a == "--delete");

    let (cfg, db, _redis) = saarctf_scripts_rs::init("sync_teams_http.rs")
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&e));
    let website_team_url = cfg
        .config
        .get("website_team_url")
        .and_then(Value::as_str)
        .unwrap_or_else(|| saarctf_scripts_rs::die("CONFIG['website_team_url'] not set"))
        .to_string();

    let mut teams: Vec<Team> = sqlx::query_as("SELECT * FROM teams")
        .fetch_all(&db)
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));

    let client = reqwest::Client::new();
    let resp = client
        .get(&website_team_url)
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("http: {e}")));
    let body: Value = resp
        .json()
        .await
        .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("http: {e}")));
    let remote_teams: Vec<Value> = body
        .get("teams")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    // Create Nop team if not existing
    let nop_team_id = cfg.scoring.nop_team_id.as_i64().unwrap_or(0);
    if nop_team_id > 0 && !teams.iter().any(|t| t.id as i64 == nop_team_id) {
        let team = Team {
            id: nop_team_id as i32,
            name: "NOP".into(),
            affiliation: None,
            website: None,
            logo: None,
            vpn_connected: false,
            vpn2_connected: false,
            vpn_connection_count: 0,
            vpn_last_connect: None,
            vpn_last_disconnect: None,
            wg_vulnbox_connected: false,
            wg_boxes_connected: false,
        };
        let _ = sqlx::query("INSERT INTO teams (id, name) VALUES ($1, $2)")
            .bind(team.id)
            .bind(&team.name)
            .execute(&db)
            .await;
        teams.push(team);
    }

    let mut teams: Vec<(i32, Option<Team>)> = teams.into_iter().map(|t| (t.id, Some(t))).collect();
    for remote in &remote_teams {
        let id = remote.get("id").and_then(Value::as_i64).unwrap_or(0) as i32;
        let get_s = |k: &str| {
            remote
                .get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if let Some(slot) = teams.iter_mut().find(|(tid, _)| *tid == id) {
            let mut team = slot.1.take().unwrap_or_else(|| Team {
                id,
                name: String::new(),
                affiliation: None,
                website: None,
                logo: None,
                vpn_connected: false,
                vpn2_connected: false,
                vpn_connection_count: 0,
                vpn_last_connect: None,
                vpn_last_disconnect: None,
                wg_vulnbox_connected: false,
                wg_boxes_connected: false,
            });
            team.name = get_s("name");
            team.affiliation = Some(get_s("affiliation"));
            team.website = Some(get_s("website"));
            import_logo_from_url(&db, &cfg, &mut team, &get_s("logo")).await;
            sqlx::query(
                "UPDATE teams SET name=$2, affiliation=$3, website=$4, logo=$5 WHERE id=$1",
            )
            .bind(id)
            .bind(&team.name)
            .bind(&team.affiliation)
            .bind(&team.website)
            .bind(&team.logo)
            .execute(&db)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
            println!("Update team \"{}\".", team.name);
        } else {
            let mut team = Team {
                id,
                name: get_s("name"),
                affiliation: Some(get_s("affiliation")),
                website: Some(get_s("website")),
                logo: None,
                vpn_connected: false,
                vpn2_connected: false,
                vpn_connection_count: 0,
                vpn_last_connect: None,
                vpn_last_disconnect: None,
                wg_vulnbox_connected: false,
                wg_boxes_connected: false,
            };
            sqlx::query(
                "INSERT INTO teams (id, name, affiliation, website) VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(&team.name)
            .bind(&team.affiliation)
            .bind(&team.website)
            .execute(&db)
            .await
            .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
            import_logo_from_url(&db, &cfg, &mut team, &get_s("logo")).await;
            sqlx::query("UPDATE teams SET logo=$2 WHERE id=$1")
                .bind(id)
                .bind(&team.logo)
                .execute(&db)
                .await
                .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
            println!("Added  team \"{}\".", team.name);
        }
    }

    // handle teams in this DB that are not present in the remote
    for (id, team) in teams.iter_mut() {
        let id = *id;
        let Some(mut team) = team.take() else {
            continue;
        };
        if id as i64 == nop_team_id {
            import_logo(&db, &cfg, &mut team).await;
            sqlx::query("UPDATE teams SET logo=$2 WHERE id=$1")
                .bind(id)
                .bind(&team.logo)
                .execute(&db)
                .await
                .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
        } else if should_delete {
            sqlx::query("DELETE FROM teams WHERE id=$1")
                .bind(id)
                .execute(&db)
                .await
                .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
            println!("Deleted team \"{}\".", team.name);
        } else {
            println!(
                "Team not found on website: #{} \"{}\". Use --delete to drop.",
                team.id, team.name
            );
            import_logo(&db, &cfg, &mut team).await;
            sqlx::query("UPDATE teams SET logo=$2 WHERE id=$1")
                .bind(id)
                .bind(&team.logo)
                .execute(&db)
                .await
                .unwrap_or_else(|e| saarctf_scripts_rs::die(&format!("db: {e}")));
        }
    }
}
