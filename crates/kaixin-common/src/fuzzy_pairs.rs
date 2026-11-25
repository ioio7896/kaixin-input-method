#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuzzyPairs {
    pub zh_z: bool,
    pub ch_c: bool,
    pub sh_s: bool,
    pub n_l: bool,
    pub f_h: bool,
    pub an_ang: bool,
    pub en_eng: bool,
    pub in_ing: bool,
}

impl Default for FuzzyPairs {
    fn default() -> Self {
        Self {
            zh_z: true,
            ch_c: true,
            sh_s: true,
            n_l: true,
            f_h: true,
            an_ang: true,
            en_eng: true,
            in_ing: true,
        }
    }
}
