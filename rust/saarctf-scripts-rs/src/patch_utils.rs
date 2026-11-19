//! Port of `controlserver/patch_utils.py` — ansible playbook/inventory
//! generation and patch publishing for the vulnbox patch scripts.

use controlserver_rs::models::Team;
use saarctf_commons_rs::config::Config;
use sqlx::PgPool;
use std::path::{Path, PathBuf};

const ANSIBLE_TEMPLATE_1: &str = "\n---\n- hosts: vulnboxes\n  tasks:\n";
const ANSIBLE_TEMPLATE_2: &str = "\n    - name: Copy <FNAME>\n      copy: # more information on this module can be found here https://docs.ansible.com/ansible/latest/modules/copy_module.html\n        # The src is the local filepath (relative from the playbook location)\n        src: \"<PATH1>\"\n        # dest is the remote path where the file should be stored\n        dest: \"/opt/<PATH2>\"\n        # content can be used instead of src to write directly into a new file. A local file can also be used as input by using functions from the templating engine (e.g. {{ lookup('file', '<local_file>') }})\n        # content: \"string\"\n        # owner: <user>\n        # group: <group>\n        mode: \"<MODE>\"\n        # if a file is overwritten create a backup of the old one first\n        # backup: yes\n";
const ANSIBLE_TEMPLATE_3: &str = "\n\n    - name: Run a single command\n      # Combining commands via pipe, etc. does not work, use the shell module for that instead: https://docs.ansible.com/ansible/latest/modules/shell_module.html#shell-module (see example below)\n      # become: yes\n      command: <COMMAND>\n      # more information on this module can be found here https://docs.ansible.com/ansible/latest/modules/command_module.html\n      # register the output of the command.\n      register: var_name\n      # indicate when the status should show 'changed'\n      changed_when: \"'DONE' not in var_name\"\n#    - name: Display the command output\n#      # use one of the following\n#      # .msg most of the time shows all outputs\n#      debug: var=var_name.msg\n#      debug: var=var_name.stdout\n#      debug: var=var_name.stderr\n\n     # shell module example:\n#    - name: Remove low sized ssh moduli\n#      shell: awk '$5 >= 4095' /etc/ssh/moduli > /etc/ssh/moduli.safe && mv /etc/ssh/moduli.safe /etc/ssh/moduli\n\n#    - name: Manage some packages\n#      # you can also write tasks as blocks in order to specify some general settings for all of them once or to make the playbook easier to read\n#      # run all tasks in the block with sudo\n#      become: yes\n#      block:\n#        - name: Install python3\n#          apt: # more information on this module can be found here https://docs.ansible.com/ansible/latest/modules/apt_module.html\n#            name: python3\n#            # absent to uninstall a package\n#            state: present\n";
const ANSIBLE_HOSTS_TEMPLATE: &str = "\n---\nall:\n  vars:\n    ansible_user: root  # specify the username used by ansible to connect to remote hosts\n    ansible_ssh_common_args: -o StrictHostKeyChecking=no\n    ansible_ssh_private_key_file: ~/.ssh/vulnbox\n  children:\n    vulnboxes:\n      hosts:\n        # 10.[32:33].[0:255].2:\n        # 127.0.0.1: {ansible_port: 22222}";

pub struct PatchUtils {
    pub patchfile: PathBuf,
    pub additional_files: Vec<PathBuf>,
    pub cfg: Config,
}

/// Python's `oct(mode & 0o777)[2:]`.
fn mode_oct(path: &Path) -> Result<String, String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .map_err(|e| e.to_string())?
        .permissions()
        .mode();
    Ok(format!("{:o}", mode & 0o777))
}

/// Python's `os.path.abspath` (no symlink resolution, unlike canonicalize).
fn abspath(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(path))
    }
}

impl PatchUtils {
    pub fn new(cfg: &Config, patchfile: PathBuf, additional_files: Vec<PathBuf>) -> PatchUtils {
        let mut patchfile = patchfile;
        if !patchfile.exists() {
            patchfile = Path::new(&cfg.patches_path).join(patchfile);
        }
        let additional_files = additional_files
            .into_iter()
            .map(|f| {
                if f.exists() {
                    f
                } else {
                    Path::new(&cfg.patches_path).join(f)
                }
            })
            .collect();
        PatchUtils {
            patchfile,
            additional_files,
            cfg: cfg.clone(),
        }
    }

    pub fn ansible_base_template() -> String {
        format!(
            "{ANSIBLE_TEMPLATE_1}{}{ANSIBLE_TEMPLATE_3}",
            ANSIBLE_TEMPLATE_2.replace("<FNAME>", "your script")
        )
    }

    pub fn ansible_filename(&self) -> PathBuf {
        self.patchfile
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!(
                "ansible-{}",
                self.patchfile
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ))
    }

    /// Generate a playbook if it does not exist; returns its filename.
    pub fn create_ansible_template(&self) -> Result<PathBuf, String> {
        let fname = self.ansible_filename();
        if !fname.exists() {
            let mode = mode_oct(&self.patchfile)?;
            let mut tmpl = format!(
                "{ANSIBLE_TEMPLATE_1}{}",
                ANSIBLE_TEMPLATE_2
                    .replace("<PATH1>", &abspath(&self.patchfile)?.to_string_lossy())
                    .replace(
                        "<PATH2>",
                        &self
                            .patchfile
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                    .replace(
                        "<FNAME>",
                        &self
                            .patchfile
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                    .replace("<MODE>", &mode)
            );
            for f in &self.additional_files {
                let mode = mode_oct(f)?;
                tmpl += &ANSIBLE_TEMPLATE_2
                    .replace("<PATH1>", &abspath(f)?.to_string_lossy())
                    .replace(
                        "<PATH2>",
                        &f.file_name().unwrap_or_default().to_string_lossy(),
                    )
                    .replace(
                        "<FNAME>",
                        &f.file_name().unwrap_or_default().to_string_lossy(),
                    )
                    .replace("<MODE>", &mode);
            }
            let mut cmd = format!(
                "/opt/{}",
                self.patchfile
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            );
            if cmd.ends_with(".sh") {
                cmd = format!("bash {cmd}");
            }
            if cmd.ends_with(".py") {
                cmd = format!("python3 {cmd}");
            }
            tmpl += &ANSIBLE_TEMPLATE_3.replace("<COMMAND>", &cmd);
            std::fs::write(&fname, tmpl).map_err(|e| e.to_string())?;
        }
        Ok(fname)
    }

    /// (all online vulnboxes, NOP-team-only) inventory file contents.
    pub async fn get_ansible_hosts_templates(
        &self,
        db: &PgPool,
    ) -> Result<(String, String), String> {
        let online_teams: Vec<Team> = sqlx::query_as(
            "SELECT * FROM teams \
             WHERE vpn_connected OR vpn2_connected OR wg_vulnbox_connected ORDER BY id",
        )
        .fetch_all(db)
        .await
        .map_err(|e| e.to_string())?;
        let mut tmpl = ANSIBLE_HOSTS_TEMPLATE.to_string();
        let mut tmpl2 = ANSIBLE_HOSTS_TEMPLATE.to_string();
        let nop = self.cfg.scoring.nop_team_id.as_i64().unwrap_or(-1);
        for team in online_teams {
            let ip = self.cfg.network.team_id_to_vulnbox_ip(team.id as i64);
            tmpl += &format!("\n        {ip}:  # {}", team.name);
            if team.id as i64 == nop {
                tmpl2 += &format!("\n        {ip}:  # {}", team.name);
            }
        }
        Ok((tmpl, tmpl2))
    }

    /// Generate fresh inventory files for all online vulnboxes; returns the
    /// filename of the primary inventory file.
    pub async fn create_ansible_hosts_template(&self, db: &PgPool) -> Result<PathBuf, String> {
        let (tmpl, tmpl2) = self.get_ansible_hosts_templates(db).await?;
        let fname = Path::new(&self.cfg.patches_path).join("hosts.yaml");
        std::fs::write(&fname, tmpl).map_err(|e| e.to_string())?;
        std::fs::write(
            Path::new(&self.cfg.patches_path).join("hosts_nop.yaml"),
            tmpl2,
        )
        .map_err(|e| e.to_string())?;
        Ok(fname)
    }

    /// Copy the patch files into the public patches dir; returns download URLs.
    pub fn publish_patch_files(&self) -> Result<Vec<String>, String> {
        let public_path = self
            .cfg
            .patches_public_path
            .as_ref()
            .ok_or_else(|| "Not configured: PATCHES_PUBLIC_PATH".to_string())?;
        std::fs::create_dir_all(public_path).map_err(|e| e.to_string())?;
        let url_base = self
            .cfg
            .patches_url
            .clone()
            .unwrap_or_else(|| "/patches/".to_string());
        let mut urls = Vec::new();
        for fname in std::iter::once(&self.patchfile).chain(self.additional_files.iter()) {
            let base = fname
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let target = Path::new(public_path).join(&base);
            if target.exists() {
                std::fs::remove_file(&target).map_err(|e| e.to_string())?;
            }
            std::fs::copy(fname, &target).map_err(|e| e.to_string())?;
            urls.push(format!("{}/{}", url_base, base));
        }
        Ok(urls)
    }
}
