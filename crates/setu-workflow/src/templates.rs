//! Starting points offered in the "New workflow" menu. They target
//! `tag:web` or `tag:db`; the person picks real hosts before running.

use crate::model::{Link, Step, When, Workflow};

fn wf(id: &str, name: &str, description: &str, steps: Vec<Step>, links: Vec<Link>) -> Workflow {
    let mut w = Workflow::new(id, name);
    w.description = description.into();
    w.steps = steps;
    w.links = links;
    w
}

pub fn all() -> Vec<Workflow> {
    vec![
        wf(
            "health-check",
            "Health check",
            "Uptime, disk and memory on every web server at once.",
            vec![
                Step::command("uptime", "Uptime", "uptime", &["tag:web"]).at(60.0, 60.0),
                Step::command("disk", "Disk space", "df -h /", &["tag:web"]).at(60.0, 200.0),
                Step::command("memory", "Memory", "free -m", &["tag:web"]).at(60.0, 340.0),
            ],
            vec![],
        ),
        wf(
            "rolling-restart",
            "Rolling restart with approval",
            "Check config, ask before restarting, restart one host at a time, roll back on failure.",
            vec![
                Step::command("test", "Test config", "sudo nginx -t", &["tag:web"]).at(60.0, 160.0),
                Step::approval(
                    "approve",
                    "Approve restart",
                    "Config is valid on every host. Restart nginx one host at a time?",
                )
                .at(330.0, 160.0),
                Step {
                    one_at_a_time: true,
                    timeout_seconds: Some(120),
                    ..Step::command(
                        "restart",
                        "Restart nginx",
                        "sudo systemctl restart nginx",
                        &["tag:web"],
                    )
                }
                .at(600.0, 60.0),
                Step::wait("settle", "Let it settle", 10).at(870.0, 60.0),
                Step::command(
                    "verify",
                    "Verify",
                    "curl -fsS -o /dev/null -w '%{http_code}\\n' http://localhost/",
                    &["tag:web"],
                )
                .at(1140.0, 60.0),
                Step::command(
                    "rollback",
                    "Roll back",
                    "sudo systemctl reload nginx",
                    &["tag:web"],
                )
                .at(600.0, 280.0),
            ],
            vec![
                Link::new("test", "approve", When::Success),
                Link::new("approve", "restart", When::Success),
                Link::new("restart", "settle", When::Success),
                Link::new("settle", "verify", When::Success),
                Link::new("restart", "rollback", When::Failure),
            ],
        ),
        wf(
            "db-backup",
            "Database backup",
            "Dump the database, then check the dump and clean old ones in parallel.",
            vec![
                Step::command(
                    "dump",
                    "Dump",
                    "pg_dump -Fc mydb > /var/backups/mydb-$(date +%F).dump",
                    &["tag:db"],
                )
                .at(60.0, 160.0),
                Step::command(
                    "check",
                    "Check dump",
                    "pg_restore -l /var/backups/mydb-$(date +%F).dump > /dev/null",
                    &["tag:db"],
                )
                .at(330.0, 60.0),
                Step::command(
                    "prune",
                    "Remove old dumps",
                    "find /var/backups -name 'mydb-*.dump' -mtime +14 -delete",
                    &["tag:db"],
                )
                .at(330.0, 260.0),
            ],
            vec![
                Link::new("dump", "check", When::Success),
                Link::new("dump", "prune", When::Success),
            ],
        ),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn templates_are_valid() {
        for w in super::all() {
            w.validate().unwrap_or_else(|e| panic!("{}: {e}", w.id));
        }
    }
}
