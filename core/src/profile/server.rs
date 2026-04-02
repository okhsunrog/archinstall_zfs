use super::Profile;

pub fn server_profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "minimal",
            display_name: "Minimal",
            packages: vec![],
            services: vec![],
        },
        Profile {
            name: "sshd",
            display_name: "SSH Server",
            packages: vec!["openssh"],
            services: vec!["sshd"],
        },
        Profile {
            name: "docker",
            display_name: "Docker",
            packages: vec!["docker"],
            services: vec!["docker"],
        },
        Profile {
            name: "httpd",
            display_name: "Apache",
            packages: vec!["apache"],
            services: vec!["httpd"],
        },
        Profile {
            name: "nginx",
            display_name: "Nginx",
            packages: vec!["nginx"],
            services: vec!["nginx"],
        },
        Profile {
            name: "cockpit",
            display_name: "Cockpit",
            packages: vec!["cockpit", "udisks2", "packagekit"],
            services: vec!["cockpit.socket"],
        },
        Profile {
            name: "postgresql",
            display_name: "PostgreSQL",
            packages: vec!["postgresql"],
            services: vec!["postgresql"],
        },
        Profile {
            name: "mariadb",
            display_name: "MariaDB",
            packages: vec!["mariadb"],
            services: vec!["mariadb"],
        },
    ]
}
