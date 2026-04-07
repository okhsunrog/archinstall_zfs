use super::{PostInstallStep, Profile, ProfileKind};

/// Helper to keep server profile literals concise.
fn server(
    name: &'static str,
    display_name: &'static str,
    packages: Vec<&'static str>,
    services: Vec<&'static str>,
    post_install_steps: Vec<PostInstallStep>,
) -> Profile {
    Profile {
        name,
        display_name,
        description: "",
        packages,
        services,
        user_services: Vec::new(),
        post_install_steps,
        kind: ProfileKind::Server,
    }
}

pub fn server_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "minimal",
            display_name: "Minimal",
            description: "Bare base system, nothing extra.",
            packages: Vec::new(),
            services: Vec::new(),
            user_services: Vec::new(),
            post_install_steps: Vec::new(),
            kind: ProfileKind::Minimal,
        },
        server(
            "sshd",
            "SSH Server",
            vec!["openssh"],
            vec!["sshd"],
            Vec::new(),
        ),
        server(
            "docker",
            "Docker",
            vec!["docker"],
            vec!["docker"],
            // Add every installer-created user to the docker group so they
            // can use Docker without sudo.
            vec![PostInstallStep::AddUsersToGroup { group: "docker" }],
        ),
        server("httpd", "Apache", vec!["apache"], vec!["httpd"], Vec::new()),
        server("nginx", "Nginx", vec!["nginx"], vec!["nginx"], Vec::new()),
        server(
            "cockpit",
            "Cockpit",
            vec!["cockpit", "udisks2", "packagekit"],
            vec!["cockpit.socket"],
            Vec::new(),
        ),
        server(
            "postgresql",
            "PostgreSQL",
            vec!["postgresql"],
            vec!["postgresql"],
            // The postgresql package creates the `postgres` system user via
            // its install scripts. initdb runs after that as the postgres
            // user to initialise the data directory. Failure is warn-only so
            // idempotent reinstalls don't abort if the directory already
            // exists.
            vec![PostInstallStep::RunAsUser {
                user: "postgres",
                cmd: "initdb",
                args: &["-D", "/var/lib/postgres/data"],
            }],
        ),
        server(
            "mariadb",
            "MariaDB",
            vec!["mariadb"],
            vec!["mariadb"],
            vec![PostInstallStep::RunAsRoot {
                cmd: "mariadb-install-db",
                args: &["--user=mysql", "--basedir=/usr", "--datadir=/var/lib/mysql"],
            }],
        ),
        server(
            "lighttpd",
            "Lighttpd",
            vec!["lighttpd"],
            vec!["lighttpd"],
            Vec::new(),
        ),
        server(
            "tomcat",
            "Tomcat",
            vec!["tomcat10", "java-runtime"],
            vec!["tomcat10"],
            Vec::new(),
        ),
    ]
}
