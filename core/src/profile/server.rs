use super::{PostInstallStep, Profile};

pub fn server_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "minimal",
            display_name: "Minimal",
            ..Profile::default()
        },
        Profile {
            name: "sshd",
            display_name: "SSH Server",
            packages: vec!["openssh"],
            services: vec!["sshd"],
            ..Profile::default()
        },
        Profile {
            name: "docker",
            display_name: "Docker",
            packages: vec!["docker"],
            services: vec!["docker"],
            // Add every installer-created user to the docker group so they can
            // use Docker without sudo.
            post_install_steps: vec![PostInstallStep::AddUsersToGroup { group: "docker" }],
            ..Profile::default()
        },
        Profile {
            name: "httpd",
            display_name: "Apache",
            packages: vec!["apache"],
            services: vec!["httpd"],
            ..Profile::default()
        },
        Profile {
            name: "nginx",
            display_name: "Nginx",
            packages: vec!["nginx"],
            services: vec!["nginx"],
            ..Profile::default()
        },
        Profile {
            name: "cockpit",
            display_name: "Cockpit",
            packages: vec!["cockpit", "udisks2", "packagekit"],
            services: vec!["cockpit.socket"],
            ..Profile::default()
        },
        Profile {
            name: "postgresql",
            display_name: "PostgreSQL",
            packages: vec!["postgresql"],
            services: vec!["postgresql"],
            // The postgresql package creates the `postgres` system user via its
            // install scripts (run by pacman during libalpm install). initdb
            // must run after that as the postgres user to initialise the data
            // directory. Failure is warn-only so idempotent reinstalls don't
            // abort if the directory already exists.
            post_install_steps: vec![PostInstallStep::RunAsUser {
                user: "postgres",
                cmd: "initdb",
                args: &["-D", "/var/lib/postgres/data"],
            }],
            ..Profile::default()
        },
        Profile {
            name: "mariadb",
            display_name: "MariaDB",
            packages: vec!["mariadb"],
            services: vec!["mariadb"],
            post_install_steps: vec![PostInstallStep::RunAsRoot {
                cmd: "mariadb-install-db",
                args: &["--user=mysql", "--basedir=/usr", "--datadir=/var/lib/mysql"],
            }],
            ..Profile::default()
        },
        Profile {
            name: "lighttpd",
            display_name: "Lighttpd",
            packages: vec!["lighttpd"],
            services: vec!["lighttpd"],
            ..Profile::default()
        },
        Profile {
            name: "tomcat",
            display_name: "Tomcat",
            packages: vec!["tomcat10", "java-runtime"],
            services: vec!["tomcat10"],
            ..Profile::default()
        },
    ]
}
