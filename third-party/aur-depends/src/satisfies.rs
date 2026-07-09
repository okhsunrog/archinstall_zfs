use alpm::{Dep, Depend, Ver, Version};
use alpm_utils::depends;

pub trait Satisfies {
    fn satisfies_dep(&self, dep: &Dep, nover: bool) -> bool {
        self.which_satisfies_dep(dep, nover).is_some()
    }

    fn which_satisfies_dep(&self, dep: &Dep, nover: bool) -> Option<&str>;
}

impl Satisfies for raur::Package {
    fn which_satisfies_dep(&self, dep: &Dep, nover: bool) -> Option<&str> {
        let provides = self.provides.iter().map(|p| Depend::new(p.as_str()));
        satisfies(
            dep,
            &self.name,
            Version::new(&*self.version),
            provides,
            nover,
        )
        .then_some(self.name.as_str())
    }
}

impl Satisfies for alpm::Package {
    fn which_satisfies_dep(&self, dep: &Dep, nover: bool) -> Option<&str> {
        satisfies(
            dep,
            self.name(),
            self.version(),
            self.provides().iter(),
            nover,
        )
        .then(|| self.name())
    }
}

impl Satisfies for srcinfo::Srcinfo {
    fn which_satisfies_dep(&self, dep: &Dep, nover: bool) -> Option<&str> {
        self.pkgs.iter().find_map(|pkg| {
            let provides = pkg
                .provides
                .iter()
                .flat_map(|p| p.values())
                .map(|p| Depend::new(p.as_str()));

            satisfies(
                dep,
                &pkg.pkgname,
                Version::new(&*self.version()),
                provides,
                nover,
            )
            .then_some(pkg.pkgname.as_str())
        })
    }
}

impl Satisfies for (&srcinfo::Srcinfo, &srcinfo::Package) {
    fn which_satisfies_dep(&self, dep: &Dep, nover: bool) -> Option<&str> {
        let provides = self
            .1
            .provides
            .iter()
            .flat_map(|p| p.values())
            .map(|p| Depend::new(p.as_str()));

        satisfies(
            dep,
            &self.1.pkgname,
            Version::new(&*self.0.version()),
            provides,
            nover,
        )
        .then_some(self.1.pkgname.as_str())
    }
}

pub fn satisfies<D: AsRef<Dep>, S: AsRef<str>, V: AsRef<Ver>>(
    dep: &Dep,
    name: S,
    version: V,
    provides: impl Iterator<Item = D>,
    nover: bool,
) -> bool {
    if nover {
        depends::satisfies_nover(dep, name, provides)
    } else {
        depends::satisfies(dep, name, version, provides)
    }
}

pub fn satisfies_provide(dep: &Dep, provide: &Dep, nover: bool) -> bool {
    if nover {
        depends::satisfies_provide_nover(dep, provide)
    } else {
        depends::satisfies_provide(dep, provide)
    }
}
