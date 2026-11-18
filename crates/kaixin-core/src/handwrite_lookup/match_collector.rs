use super::Match;

pub struct MatchCollector<'a> {
    limit: usize,
    matches: &'a mut Vec<Match>,
}

impl<'a> MatchCollector<'a> {
    pub fn new(matches: &mut Vec<Match>, limit: usize) -> MatchCollector {
        assert!(
            limit > 0,
            "Expected a positive number for the maximum number of matches."
        );
        assert!(
            matches.is_empty(),
            "The pre-existing matches vector must be empty."
        );
        MatchCollector { limit, matches }
    }

    fn remove_existing_lower(&mut self, mc: &Match) -> bool {
        let mut ix: i32 = -1;
        for i in 0..self.matches.len() {
            if self.matches[i].hanzi == mc.hanzi {
                ix = i as i32;
                break;
            }
        }
        // Not there yet: we're good, match doesn't need to be skipped
        if ix == -1 {
            return false;
        }
        // New score is not better: skip new match
        if mc.score <= self.matches[ix as usize].score {
            return true;
        }
        // Remove existing match; don't skip new. Means shifting array left.
        self.matches.remove(ix as usize);
        false
    }

    pub fn file_match(&mut self, mc: Match) {
        // Already at limit: don't bother if new match's score is smaller than current minimum
        if self.matches.len() == self.limit && mc.score <= self.matches.last().unwrap().score {
            return;
        }
        // Remove if we already have this character with a lower score
        // If we get "true", we should skip new match (already there with higher score)
        if self.remove_existing_lower(&mc) {
            return;
        }
        // Where does new match go? (Keep array sorted largest score to smallest.)
        // Largest score is always at start of vector.
        let ix = self.matches.iter().position(|x| x.score < mc.score);
        match ix {
            Some(ix) => self.matches.insert(ix, mc),
            None => self.matches.push(mc),
        }
        // Beyond limit? Drop last item.
        if self.matches.len() > self.limit {
            self.matches.pop();
        }
    }
}
