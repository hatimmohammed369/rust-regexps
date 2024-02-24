// Use a parsed regular expression to match against strings

use crate::parser::{parse, syntax_tree::*};

// Match operation outcome
pub type Match = std::ops::Range<usize>;

#[allow(dead_code)]
// If an expression E can backtrack (like a+)
// then each time it successfully matches a range
// record that range such that if it needs to backtrack
// Matcher can use its last record range to force it
// to match a smaller range
struct ExpressionBacktrackInfo {
    // The last item in this Vec represent the index of current pattern among its siblings
    // within its level in parsed pattern syntax tree
    // All other items represent the index of its parents among their siblings within
    // the their respective parsed pattern syntax tree level
    index_sequence: Vec<usize>,

    // Position of first successful match of the associated expression
    // This field is never mutated once set
    last_match_start: usize,

    // Upper exclusive bound next match MUST satisfy
    last_match_end: usize,

    backtracked_to_last_match_start: bool,
}

// Coordinator of the matching process
pub struct Matcher {
    // Currently processed node of the given pattern syntax tree
    pattern: Regexp,

    // String on which the search (pattern matching) is done
    target: Vec<char>,

    // Where to start matching
    current: usize,

    // Which index the currently computed match begins at
    current_match_start: usize,
    // Expressions are not allowed to backtrack before `current_match_start`
    // to keep generated matches non-overlapping

    // True if Matcher generated an empty string match in current position
    // False otherwise
    matched_empty_string: bool,

    // The last item in this Vec represent the index of current pattern among its siblings in
    // current syntax tree level
    // All other items represent the index of its parents amongs their siblings with the same
    // syntax tree level
    pattern_index_sequence: Vec<usize>,
    // Of course, root pattern (currently processed pattern) will have Vec
    // of one 0usize item, because root has no parent and its the zeroth child in its level
    // For instance, a value of X = vec![0, 3, 4] means that currently processed pattern (subexpression)
    // is the fourth (X[2]) child within its level
    // its parent is the third(X[1]) child within the level above
    // its grandparent is the root (X[0])

    // Backtrack info of all subexpressions which can backtrack
    backtrack_table: Vec<ExpressionBacktrackInfo>,
}

impl Matcher {
    // Create a new matcher from `pattern`
    // which is matched against `target`
    pub fn new(pattern: &str, target: &str) -> Result<Matcher, String> {
        let pattern = parse(pattern)?;
        let target = target.chars().collect();
        let current = 0;
        let current_match_start = 0;
        let matched_empty_string = false;
        let pattern_index_sequence = vec![];
        let backtrack_table = vec![];
        Ok(Matcher {
            pattern,
            target,
            current,
            current_match_start,
            matched_empty_string,
            pattern_index_sequence,
            backtrack_table,
        })
    }

    fn has_next(&self) -> bool {
        self.current < self.target.len()
    }

    fn set_position(&mut self, pos: usize) {
        let pos = if pos > self.target.len() {
            self.target.len()
        } else {
            pos
        };

        let old_pos = self.current;
        self.current = pos;

        if old_pos < self.target.len() || !self.matched_empty_string {
            // !( old_pos == self.target.len() && self.matched_empty_string )
            // calling one of `self.set_position` or `self.advance`
            // ensures that old position (old_pos) is never greater than self.target.len()
            // so !(old_pos == self.target.len()) is never (old_pos > self.target.len())
            // hence it MUST be (old_pos < self.target.len())

            // It's NOT the case that we matched the trailing empty string
            // If we matched the trailing empty string and unset flag `matched_empty_string`
            // then Matcher would get stuck in a loop, indefinitely matching the trailing empty
            // because it setting and unsetting flag `matched_empty_string`
            self.matched_empty_string = false;
        }
    }

    fn advance(&mut self) {
        if self.current < self.target.len() {
            self.current += 1;
        }
    }

    // Assign a new target to match on
    pub fn assign_match_target(&mut self, target: &str) {
        self.target = target.chars().collect();
        self.set_position(0);
        self.pattern_index_sequence.clear();
        self.backtrack_table.clear();
    }

    // Find the next match (non-overlapping with previous match)
    pub fn find_match(&mut self) -> Option<Match> {
        // Track root expression
        self.dive();

        // WHY WE NEED A LOOP?
        // Because first match in target string may not be index 0
        // and hence we need to keep matching until we hit the first match
        // or reach end of target
        let mut match_attempt;
        loop {
            match_attempt = self.compute_match();
            // Remove backtrack info relevant last performed match
            // because we need to record bounds based on `self.current_match_start`
            self.backtrack_table.clear();
            if match_attempt.is_none() {
                // Last match failed
                if self.has_next() {
                    // Move forward to retry
                    // ADVANCE
                    self.advance();
                } else {
                    // No more characters to process
                    // HALT
                    break;
                }
            } else {
                // Return matched region
                if match_attempt.as_ref().unwrap().is_empty() {
                    // Matched the empty string in current position
                    // Matcher MUST advance or it will loop endlessly
                    // matching the empty string at the same position
                    // because the empty string can match anywhere
                    self.advance();
                    // So we don't get stuck at the same position matching the empty string forever
                    self.current_match_start += 1;
                } else {
                    // Next match starts right after current one
                    self.current_match_start = match_attempt.as_ref().unwrap().end;
                }
                break;
            }
        }

        // Abandon root expression
        self.bubble_up();

        match_attempt
    }

    fn supports_backtracking(expr: &Regexp) -> bool {
        // An arbitrary expression E supports backtracking if:
        // 1 - It's quantified, in other words it's preceeding a quantifier (like .*)
        // 2 - At least one of its children supports backtracking (like (a+|c) because a+ can backtrack)

        match &expr.tag {
            // The empty expression can match anywhere
            // It doesn't need backtracking
            ExpressionTag::EmptyExpression => false,

            ExpressionTag::CharacterExpression { quantifier, .. } => {
                // . or x are quantified
                !matches!(quantifier, Quantifier::None)
            }

            ExpressionTag::Group { quantifier } => {
                // The group itself is quantified or the grouped expression
                // inside supports backtracking
                !matches!(quantifier, Quantifier::None)
                    || Self::supports_backtracking(&expr.children.borrow()[0].borrow())
            }

            // Alternation and concatenation
            _ => {
                // At least one child supports backtracking
                expr.children
                    .borrow()
                    .iter()
                    .any(|child| Self::supports_backtracking(&child.borrow()))
            }
        }
    }

    // ALL EXPRESSIONS MUST RESTORE OLD POSITION WHEN FAILING TO MATCH
    fn compute_match(&mut self) -> Option<Match> {
        let computed_match = match self.pattern.tag.clone() {
            ExpressionTag::EmptyExpression => self.empty_expression_match(),
            ExpressionTag::CharacterExpression { value, quantifier } => {
                self.character_expression_match(value, quantifier)
            }
            ExpressionTag::Group { quantifier } => self.group_match(quantifier),
            ExpressionTag::Alternation => self.alternation_match(),
            ExpressionTag::Concatenation => self.concatenation_match(),
        };

        // Destroy last used bound
        // BUT WHY?
        // If last match is successful, then it makes no sense to make next expression
        // backtrack making current one backtrack again (loop)

        // If current expression successfully matched AND
        // It can backtrack (like .?) AND
        // It's not root expression (it makes no sense to have root expression request a backtrack, it has no siblings)
        if computed_match.is_some()
            && Self::supports_backtracking(&self.pattern)
            // Root expression does not backtrack
            && self.pattern.parent.is_some()
        {
            // THEN
            // Record first match info for later use when backtracking

            let (start, end) = {
                let temp = computed_match.as_ref().unwrap();
                (temp.start, temp.end)
            };

            // Attempt to find current expression info entry
            let search_index = self.backtrack_table.binary_search_by(|info_entry| {
                info_entry.index_sequence.cmp(&self.pattern_index_sequence)
            });
            match search_index {
                Ok(item_index) => {
                    // Found entry

                    let expr_info = &mut self.backtrack_table[item_index];

                    // Reset this entry because backtracking expressions are computed once
                    // during the first match and then used repeatedly by other matches
                    expr_info.last_match_start = start;
                    expr_info.last_match_end = end;
                    expr_info.backtracked_to_last_match_start = start == end;
                }
                Err(insertion_index) => {
                    // This expression never matched before
                    // Insert a new info entry while maintaining order of all entries
                    // Entries (ExpressionBacktrackInfo objects) are sorted by field 'index_sequence'

                    self.backtrack_table.insert(
                        insertion_index,
                        ExpressionBacktrackInfo {
                            index_sequence: self.pattern_index_sequence.clone(),
                            last_match_start: start,
                            last_match_end: end,
                            backtracked_to_last_match_start: start == end,
                            // If this subexpression just matched the empty string,
                            // it CAN NOT backtrack anymore
                        },
                    )
                }
            }
        }

        computed_match
    }

    fn dive(&mut self) {
        // Begin matching a child of current patttern
        self.pattern_index_sequence.push(0);
    }

    fn bubble_up(&mut self) {
        // Done matching current pattern, go up back to its parent
        self.pattern_index_sequence.pop();
    }

    fn appoint_next_child(&mut self) {
        // Begin matching a sibling of current pattern
        *self.pattern_index_sequence.last_mut().unwrap() += 1;
    }

    // EMPTY EXPRESSIONS:
    // "" `an empty pattern string`
    // ()
    // ...(|...)... `between ( and |`
    // ...(...|)... `between | and )`
    // |... `before the leading |`
    // ...| `after the trailing |`
    // ...||... `between the two |`

    // HOW TO MATCH AN EMPTY STRING EXPRESSION:
    // Match current position in `target` against the empty regular expression
    // this function always succeeds, returning Some(Match)
    // because the empty string always matches even inside another empty string
    // There is only one case when this function fails (return None)
    // it's when Matcher matched the trailing empty string (empty string after the last valid index)
    // it makes sense to stop there or Matcher would endlessly match that trailing empty string
    fn empty_expression_match(&mut self) -> Option<Match> {
        if !self.matched_empty_string || self.has_next() {
            // Not matched empty string here or not all characters processed
            // logical negation of: Matched trailing empty string
            // which is (self.matched_empty_string && !self.has_next())
            self.matched_empty_string = true;
            Some(Match {
                start: self.current,
                end: self.current,
            })
        } else {
            // Matched trailing empty string
            // Target string is completely consumed
            // NO MORE MATCHES FOR THIS TARGET
            None
        }
    }

    // CHARACTER & DOT EXPRESSIONS:
    // x \ x? \ x* \ x+
    // . \ .? \ .* \ .+
    // x is a single character
    // Also, x is not a metacharacter or it's an escaped metacharacter
    // metacharacters are defined in file `grammar`
    // for instance, k+ is a character expression

    // HOW TO MATCH CHARACTER & DOT EXPRESSIONS?
    // If field `value`, found in field `tag` of this expression, is Option::<char>::None
    // then this character expression is actually a dot expression
    // If this expression is `.`, consume a single character
    // If this expression `x`, consume a single `x` only if current character is `x`
    //
    // For any other character (or dot) expression, if the expression NEVER matched before
    // allow it to consume as many x's (or characters) as possible
    // If the expression DID match before, use field `last_match_end` in its associated
    // ExpressionBacktrackInfo in `self.backtrack_table`
    // Temporarily subtract one (if possible) from field `last_match_end` and use it
    // as a bound of current match
    fn character_expression_match(
        &mut self,
        value: Option<char>,
        quantifier: Quantifier,
    ) -> Option<Match> {
        // Choose match bound (where the match ends)
        // A non-backtracking expression (`.` or `x`) gets bound (self.current + 1)
        // For any other expression subtract one (if possible) from field `last_match_end`
        // found in its associated entry (ExpressionBacktrackInfo object) in self.backtrack_table
        let match_bound = {
            // Find backtrack table entry of this expression
            // Expressions `.` and `x` DO NOT have such entries, NEVER
            let table_entry = self
                .backtrack_table
                .iter()
                .find(|entry| entry.index_sequence == self.pattern_index_sequence);
            match table_entry {
                // This expression can backtrack (a quantified `.` or `x`, for instance `.+`)
                // AND also it has a backtrack table entry
                Some(info) => info.last_match_end.saturating_sub(1),

                None => {
                    // This expression NEVER matched before
                    // OR it does not support backtracking (like `x` or `.`)
                    match quantifier {
                        // Expressions `.` and `x`
                        // Consume exactly one character
                        Quantifier::None | Quantifier::ZeroOrOne => self.current + 1,

                        // Quantified `.` or `x`
                        // Consume as many characters as possible
                        _ => self.target.len(),
                    }
                }
            }
        };

        let start = self.current;
        // Consume characters as long as there are unmatched characters
        // only if this expression is a dot expression or the next unmatched
        // character is `x` and this expression is one of: x \ x? \ x* \ x+
        while self.has_next()
            && self.current < match_bound
            && !(value.is_some() && self.target[self.current] != value.unwrap())
        {
            self.advance();
        }
        let end = self.current;

        if start == end {
            // Empty range
            match quantifier {
                // Expressions . \ .+ \ x \ x+ MUST match at least one character
                // they fail otherwise
                Quantifier::None | Quantifier::OneOrMore => Option::<Match>::None,

                // Expressions .? \ .* \ x? \ x* match the empty string
                // when they fail to match at least one character
                _ => self.empty_expression_match(),
            }
        } else {
            Some(Match { start, end })
        }
    }

    // GROUP/GROUPED EXPRESSIONS:
    // (E) where E is also an expression
    // for instance, (a+|b) is group/grouped expression

    // HOW TO MATCH GROUPED EXPRESSION:
    // Match whatever grouped expression matched
    // and then apply the quantifiers after the group itself
    fn group_match(&mut self, quantifier: Quantifier) -> Option<Match> {
        let old_pattern = self.pattern.clone();
        self.pattern = old_pattern.children.borrow()[0].borrow().clone();

        let grouped_expression_mactch = {
            // Match a grouped expression

            let match_bound = {
                // Find backtrack entry (in self.backtrack_table) of this group expression
                let table_entry = self
                    .backtrack_table
                    .iter()
                    .find(|entry| entry.index_sequence == self.pattern_index_sequence);
                match table_entry {
                    // This expression matched/backtracked before
                    Some(info) => info.last_match_end.saturating_sub(1),
                    // This expression NEVER matched/backtracked before
                    None => self.target.len(),
                }
            };

            // Start tracking your child
            self.dive();
            // `self.dive()` MUST be called here because it mutates `self.pattern_index_sequence`
            // which is used to find associated entry (in self.backtrack_table) of this group itself

            let start = self.current;
            let mut end = self.current;
            // Keep matching inner expression unless match bound is exceeded
            while let Some(new_match) = self.compute_match() {
                if self.current > match_bound {
                    // Match bound exceeded while matching inner expression
                    // Roll back to end of most recent match
                    self.set_position(end);
                    break;
                }
                if new_match.is_empty() {
                    // Stop matching inner expression E when it has
                    // matched the empty string
                    break;
                }
                // New match made without exceeding match bound
                // Update match end index of this group expression
                end = new_match.end;
            }

            if start == end {
                // Empty range
                match quantifier {
                    // E failed, then so would (E) and (E)+
                    Quantifier::None | Quantifier::OneOrMore => Option::<Match>::None,

                    // E failed, then (E)? and (E)* match the empty string
                    _ => self.empty_expression_match(),
                }
            } else {
                Some(Match { start, end })
            }

            // Grouped expression computation ends
        };

        // Restore parent pattern to process remaining siblings of current pattern
        self.pattern = old_pattern;
        // Abandon your child
        self.bubble_up();

        grouped_expression_mactch
    }

    // ALTERNATION EXPRESSIONS:
    // (E1|E2|...|E_n) where E1,E2,...,E_n are also expressions
    // for instance, a|b.c|x is an alternation expression

    // HOW TO MATCH AN ALTERNATION EXPRESSION:
    // Match children in order from first to last
    // return the match of the first matching child
    fn alternation_match(&mut self) -> Option<Match> {
        // Start tracking your children
        self.dive();

        let old_position = self.current;
        let old_pattern = self.pattern.clone();

        let alternation_match = {
            // Match an alternation expression

            let children = self
                .pattern
                .children
                .borrow()
                .iter()
                .map(|rc| rc.borrow().clone())
                .collect::<Vec<_>>();
            let mut child_match = None;
            for child in children {
                self.pattern = child;
                child_match = self.compute_match();
                if child_match.is_none() {
                    // Return to original position this alternation expression started at
                    // to make all its children start matching from the same position
                    self.set_position(old_position);
                    // If last child failed to match, the above call
                    // automatically restores old position where this alternation started matching
                } else {
                    // One of the branches matched
                    // The whole alternation expression has matched
                    // Return its match
                    break;
                }
                // Start tracking next child
                self.appoint_next_child();
            }

            // Alternation expression match computation ends
            child_match
        };

        // Restore parent pattern to process remaining siblings of current pattern
        self.pattern = old_pattern;
        // Abandon your children
        self.bubble_up();

        alternation_match
    }

    // CONCATENATION EXPRESSIONS:
    // E1E2...E_n, where E1, E2, ..., E_n are also expressions
    // for instance, a.(a+|b*)c* is a concatenation expression with
    // E1 = a, E2 = ., E3 = (a+|b*), E4 = c*

    // HOW TO MATCH A CONCATENATION EXPRESSION:
    // If at least one child fails, then the whole expression fails too
    // Otherwise return the range starting from first child match
    // and ending with last child match
    fn concatenation_match(&mut self) -> Option<Match> {
        // Start tracking your children
        self.dive();

        let old_position = self.current;
        let old_pattern = self.pattern.clone();

        let concatenation_match = {
            // Match a concatenation expression

            let children = self
                .pattern
                .children
                .borrow()
                .iter()
                .map(|rc| rc.borrow().clone())
                .collect::<Vec<_>>();
            // Positions (indices) of children supporting backtrack in Vec `children`
            // and in Matcher field `backtrack_table`
            let mut backtracking_siblings_positions = Vec::<(usize, usize)>::new();
            let mut match_region_end = self.current;
            let mut child_index = 0usize;
            while child_index < children.len() {
                let child = &children[child_index];
                self.pattern = child.clone();

                match self.compute_match() {
                    Some(match_obj) => {
                        // If this expression matched, then its match begins
                        // right after the match of its predecessor
                        // that's because Matcher field `self.current`
                        // is never incremented before doing the actual matching
                        // but it's incremented after a successful match
                        match_region_end = match_obj.end;
                        if Self::supports_backtracking(&self.pattern)
                            && !backtracking_siblings_positions // do not list the same sibling twice
                                .iter()
                                .any(|(child_idx, _)| *child_idx == child_index)
                        {
                            backtracking_siblings_positions
                                .push((child_index, self.backtrack_table.len() - 1));
                        }
                    }
                    None => {
                        let nearest_preceeding_backtracking_sibling =
                            backtracking_siblings_positions
                                .iter()
                                .filter(|(sibling_index, _table_entry_index)| {
                                    // Sibling is actually before current index (child_index)
                                    // and it has NOT backtracked to current match start
                                })
                                .next_back();
                        match nearest_preceeding_backtracking_sibling.cloned() {
                            Some((sibling_index, table_entry_index)) => {
                                // Start matching from that backtracking preceeding sibling
                                child_index = sibling_index;
                                let table_entry = &self.backtrack_table[table_entry_index];
                                // Restore position when this concatenation began matching
                                self.set_position(table_entry.last_match_start);
                                // Fix subexpressions index tracker
                                *self.pattern_index_sequence.last_mut().unwrap() = child_index;
                                continue;
                            }
                            None => {
                                // An item failed to match and none of its
                                // preceeding siblings can backtrack
                                // The whole concatenation expression fails
                                // Restore parent pattern to process remaining siblings of current pattern
                                self.pattern = old_pattern;
                                // Restore old position
                                self.set_position(old_position);
                                // Abandon your children
                                self.bubble_up();
                                return None;
                            }
                        }
                    }
                }

                child_index += 1;
                // Start tracking next child
                self.appoint_next_child();
            }

            // Concatenation expression match computation ends
            Some(Match {
                start: old_position,
                end: match_region_end,
            })
        };

        self.pattern = old_pattern.clone();
        // Abandon your children
        self.bubble_up();

        concatenation_match
    }
}
