// Use a parsed regular expression to match against strings

use std::sync::{Arc, RwLock};

use crate::parser::{syntax_tree::*, Parser};

const METACHARACTERS: [char; 7] = ['(', ')', '\\', '|', '*', '.', '?'];

pub fn escape(pattern: &str) -> String {
    // Escape all metacharacters in `pattern`
    let mut escaped = String::with_capacity(
        // Possible each character is a metacharacter
        // requiring two slashes
        3 * pattern.len(),
    );
    for ch in pattern.chars() {
        if METACHARACTERS.contains(&ch) {
            // Add a slash to escaped the metacharacter
            // You need to write one slash BUT Rust needs you to escape this one slash
            // so actually we need 2 slashes
            escaped.push('\\'); // Rust escaping slash
            escaped.push('\\'); // ParsedRegexp escaping slash
        }
        escaped.push(ch);
    }
    escaped.shrink_to_fit();
    escaped
}

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

    // Position of last successful match of the associated expression
    last_match_start: usize,

    // End position of last successful match of the associated expression
    last_match_end: usize,

    // True if the associated expression backtracked until it attempted to
    // match with the range starting and ending at `last_match_start`
    // False otherwise
    backtracked_to_last_match_start: bool,
    // If true, reset fields of this ExpressionBacktrackInfo unless its
    // associated expression has NO preceeding backtrackable sibling
    // with its respective field `backtracked_to_last_match_start` set false
    // If it has no such sibling then its parent (a concatenation) fails to match
}

#[derive(Debug, Clone, Copy)]
enum MatchPhase {
    Normal,
    TrailingEmptyString,
    Finished,
}

// Coordinator of the matching process
pub struct Matcher {
    // Currently processed node of the given pattern syntax tree
    pattern: Arc<RwLock<ParsedRegexp>>,

    // String on which the search (pattern matching) is done
    target: Vec<char>,
    // Direct indexing, not supported by String, is usually needed
    // so it's better to use a Vec rather than a String

    // Current position in target string (Vec field `target`)
    pos: usize,

    next_match_phase: MatchPhase,

    // The last item in this Vec represent the index of current pattern among its siblings in
    // current syntax tree level
    // All other items represent the index of its parents amongs their siblings with the same
    // syntax tree level
    pattern_index_sequence: Vec<usize>,
    // Of course, root pattern (ParsedRegexp parsed in Matcher::new) will have Vec
    // of one 0usize item, because root has no parent and its the zeroth (first) child in its level
    // For instance, a value of X = vec![0, 3, 4] means that currently processed pattern (subexpression)
    // is the fourth (X[2]) child within its level
    // its parent is the third(X[1]) child within the level above
    // its grandparent is the root (X[0])

    // Backtrack info of all subexpressions which can backtrack
    backtrack_table: Vec<ExpressionBacktrackInfo>,

    // Exclusive upper bound of ongoing match
    // Match fails if gives back a range whose end index >= `match_bound`
    match_bound: usize,

    // Successful matches
    match_cache: Vec<Match>,

    // Target substring containing all matches start index
    matches_substring_start: Option<usize>,

    // Target substring containing all matches end index
    matches_substring_end: usize,
}

impl Matcher {
    // Create a new matcher from `pattern`
    // which is matched against `target`
    pub fn new(pattern: &str, target: &str) -> Result<Matcher, String> {
        let pattern = Parser::parse(pattern)?;
        let target = target.chars().collect::<Vec<_>>();
        let pos = 0;
        let next_match_phase = MatchPhase::Normal;
        let pattern_index_sequence = vec![];
        let backtrack_table = vec![];
        let match_bound = target.len() + 1;
        let match_cache = vec![];
        let matches_substring_start = Option::<usize>::None;
        let matches_substring_end = 0;

        Ok(Matcher {
            pattern,
            target,
            pos,
            next_match_phase,
            pattern_index_sequence,
            backtrack_table,
            match_bound,
            match_cache,
            matches_substring_start,
            matches_substring_end,
        })
    }

    // Current "normalized" position
    // Always return something less than or equal to target length
    #[inline(always)]
    fn current(&self) -> usize {
        std::cmp::min(self.pos, self.target.len())
    }

    #[inline(always)]
    fn has_next(&self) -> bool {
        self.pos < self.target.len()
    }

    #[inline(always)]
    fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }

    #[inline(always)]
    fn advance(&mut self) {
        self.pos += 1;
    }

    // Assign a new target to match on
    pub fn assign_match_target(&mut self, target: &str) {
        self.target = target.chars().collect();
        self.match_cache.clear();
        self.reset();
    }

    // Assign a new pattern to match against
    pub fn assign_pattern_string(&mut self, pattern: &str) -> Result<(), String> {
        self.pattern = Parser::parse(pattern)?;
        self.match_cache.clear();
        self.reset();
        Ok(())
    }

    // Assign a new pattern to match against
    pub fn assign_pattern_regexp(&mut self, regexp: &Arc<RwLock<ParsedRegexp>>) {
        self.pattern = {
            let regexp = regexp.read().unwrap();
            regexp.deep_copy()
        };
        self.match_cache.clear();
        self.reset();
    }

    // Reset state and use old pattern
    pub fn reset(&mut self) {
        self.seek(0);
    }

    pub fn seek(&mut self, position: usize) {
        // Rewind
        self.set_position(position);
        // Back to normal matching mode (processing target)
        self.next_match_phase = MatchPhase::Normal;
        // Stop tracking expressions
        self.pattern_index_sequence.clear();
        // Do not use old backtrack info
        self.backtrack_table.clear();
    }

    fn supports_backtracking(expr: &Arc<RwLock<ParsedRegexp>>) -> bool {
        // An arbitrary expression E supports backtracking if:
        // 1 - It's quantified, in other words it's succeeded by a quantifier, like `.*`
        // 2 - At least one of its children supports backtracking, like `(a+|c)` because a+ can backtrack

        let parsed_expr = expr.read().unwrap();
        let expr_type = parsed_expr.expression_type;
        match expr_type {
            // The empty expression can match anywhere
            // It doesn't need backtracking
            ExpressionType::EmptyExpression => false,

            ExpressionType::CharacterExpression { quantifier, .. } => {
                // . or x are quantified

                // It's not the case that this expression has no quantifier
                // in other words, it's quantified with one of ? \ * \ +
                !matches!(quantifier, Quantifier::None)
                // Variant Quantifier::None represent the idea of `no quantifier`
            }

            ExpressionType::Group { quantifier } => {
                // The group itself is quantified or the grouped expression
                // inside supports backtracking

                // It's not the case that this expression has no quantifier
                // in other words, it's quantified with one of ? \ * \ +
                !matches!(quantifier, Quantifier::None)
                    || Self::supports_backtracking(&parsed_expr.children.read().unwrap()[0])
                // Variant Quantifier::None represent the idea of `no quantifier`
            }

            // Alternation and concatenation
            _ => {
                // At least one child supports backtracking
                parsed_expr
                    .children
                    .read()
                    .unwrap()
                    .iter()
                    .any(Self::supports_backtracking)
            }
        }
    }

    // ALL EXPRESSIONS MUST RESTORE OLD POSITION WHEN FAILING TO MATCH
    fn compute_match(&mut self) -> Option<Match> {
        let parsed_pattern = Arc::clone(&self.pattern);
        let parsed_pattern = parsed_pattern.read().unwrap();
        let pattern_type = parsed_pattern.expression_type;

        let computed_match = match pattern_type {
            ExpressionType::EmptyExpression => self.empty_expression_match(),

            ExpressionType::CharacterExpression { value, quantifier } => {
                self.character_expression_match(value, quantifier)
            }

            ExpressionType::Group { quantifier } => self.group_match(quantifier),

            ExpressionType::Alternation => self.alternation_match(),
            ExpressionType::Concatenation => self.concatenation_match(),
        };

        // Grouped expressions do not have entries in backtrack table `self.backtrack_table`
        // but they MUST never give back a match whose end index >= match bound of their group parent
        let expression_not_grouped = {
            match &parsed_pattern.parent {
                Some(parent_weak_ref) => {
                    let parent = parent_weak_ref.upgrade().unwrap();
                    let parent_is_a_group = matches!(
                        parent.read().unwrap().expression_type,
                        ExpressionType::Group { .. }
                    );
                    !parent_is_a_group
                }
                None => {
                    // No parent => no group parent
                    true // that this expression is not grouped
                }
            }
        };

        // If current expression successfully matched AND
        // It can backtrack (like .?) AND
        // It's not root expression (it makes no sense to have root expression request a backtrack, it has no siblings)
        if computed_match.is_some()
            && Self::supports_backtracking(&self.pattern)
            // Root expression does not backtrack
            && parsed_pattern.parent.is_some()
            && expression_not_grouped
        {
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
                    // Reset `last_match_start` to make the associated expression of this entry usable
                    expr_info.last_match_start = start;
                    // Update other values
                    expr_info.last_match_end = end;
                    // When matching, expression `last_match_end - 1` is used as current bound match
                    // so if the expression made a match, variable `end` will have smaller value
                    // than field `last_match_end` because end it's at most (last_match_end - 1)
                    expr_info.backtracked_to_last_match_start = start == end;
                }
                Err(insertion_index) => {
                    // This expression never matched before
                    // Insert a new info entry while maintaining order of all entries
                    // Insert at index found by binary search stored in `search_index`
                    // Entries (ExpressionBacktrackInfo objects) are sorted by field 'index_sequence'

                    self.backtrack_table.insert(
                        insertion_index,
                        ExpressionBacktrackInfo {
                            index_sequence: self.pattern_index_sequence.clone(),
                            last_match_start: start,
                            last_match_end: end,
                            backtracked_to_last_match_start: start == end,
                        },
                    )
                }
            }
        }

        computed_match
    }

    #[inline(always)]
    fn dive(&mut self) {
        // Begin matching a child of current patttern
        self.pattern_index_sequence.push(0);
    }

    #[inline(always)]
    fn bubble_up(&mut self) {
        // Done matching current pattern, go up back to its parent
        self.pattern_index_sequence.pop();
    }

    #[inline(always)]
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

    // Always match
    #[inline(always)]
    fn empty_expression_match(&mut self) -> Option<Match> {
        // BUT ensure Matcher advances (call self.advance) later
        // otherwise Matcher would loop endlessly matching the empty string
        // at the same position because empty expression match NEVER fails
        let current = self.current();
        Some(Match {
            start: current,
            end: current,
        })
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

    // For any other backtracking character (or dot) expression (a . or an arbitrary character followed by one of ? \ * \ +)
    // if the expression NEVER matched before
    // allow it to consume as many x's (or characters) as possible
    // If the expression DID match before, use field `last_match_end` in its associated
    // ExpressionBacktrackInfo in `self.backtrack_table` to force it to match a smaller range
    // Temporarily subtract one (if possible) from field `last_match_end` and use it
    // as a bound of current match

    // Return Option::<std::ops::Range>::Some(...) on success
    // Return Option::<std::ops::Range>::None on failure
    fn character_expression_match(
        &mut self,
        value: Option<char>,
        quantifier: Quantifier,
    ) -> Option<Match> {
        let old_match_bound = self.match_bound;
        self.match_bound = {
            // Find backtrack entry (in self.backtrack_table) of this character/dot expression
            let table_entry_index = self.backtrack_table.binary_search_by(|info_entry| {
                info_entry.index_sequence.cmp(&self.pattern_index_sequence)
            });
            match table_entry_index {
                // This expression matched/backtracked before
                Ok(entry_index) => {
                    // Subtract one, if possible, from last match end index
                    // to force this expression to match a smaller range
                    self.backtrack_table[entry_index]
                        .last_match_end
                        .saturating_sub(1)
                }
                // This expression NEVER matched/backtracked before
                _ => old_match_bound,
            }
        };

        let expr_match = match quantifier {
            Quantifier::None | Quantifier::ZeroOrOne => {
                // Match `x`\`x?` (value = Some('x')) or `.`\`.?` (value = None)
                if self.has_next() && (value.is_none() || self.target[self.pos] == value.unwrap()) {
                    Option::<Match>::Some(Match {
                        start: self.current(),
                        end: {
                            self.advance();
                            self.current()
                        },
                    })
                } else if matches!(quantifier, Quantifier::None) {
                    Option::<Match>::None
                } else {
                    self.empty_expression_match()
                }
            }

            _ => {
                // Match `x*` \ `x+` (value = Some('x')) or `.*` \ `.+` (value = None)
                let start = self.current();
                if value.is_none() {
                    // Matching `.*` or `.+`
                    // Just move `self.pos`
                    self.set_position(self.match_bound.saturating_sub(1));
                } else {
                    let value = value.unwrap();
                    while let Some(target_char) = self.target.get(self.pos) {
                        if *target_char != value || self.pos >= self.match_bound {
                            break;
                        }
                        self.advance();
                    }
                }
                let end = self.current();

                if start < end {
                    Option::<Match>::Some(Match { start, end })
                } else if matches!(quantifier, Quantifier::ZeroOrMore) {
                    self.empty_expression_match()
                } else {
                    // Match bound exceeded/reached, abort
                    Option::<Match>::None
                }
            }
        };

        self.match_bound = old_match_bound;

        expr_match
    }

    // GROUP/GROUPED EXPRESSIONS:
    // (E) where E is also an expression
    // for instance, (a+|b) is group/grouped expression

    // HOW TO MATCH GROUPED EXPRESSION:
    // Match whatever grouped expression matched
    // and then apply the quantifiers after the group itself

    // Return Option::<std::ops::Range>::Some(...) on success
    // Return Option::<std::ops::Range>::None on failure
    fn group_match(&mut self, quantifier: Quantifier) -> Option<Match> {
        let old_match_bound = self.match_bound;
        self.match_bound = {
            // Find backtrack entry (in self.backtrack_table) of this group expression
            let table_entry_index = self.backtrack_table.binary_search_by(|info_entry| {
                info_entry.index_sequence.cmp(&self.pattern_index_sequence)
            });
            match table_entry_index {
                // This expression matched/backtracked before
                Ok(entry_index) => self.backtrack_table[entry_index]
                    .last_match_end
                    .saturating_sub(1),
                // This expression NEVER matched/backtracked before
                _ => old_match_bound,
            }
        };

        let old_pattern = Arc::clone(&self.pattern);
        let pattern = Arc::clone(&old_pattern);
        let pattern = pattern.read().unwrap();
        let pattern = &pattern.children;
        self.pattern = Arc::clone(&pattern.read().unwrap()[0]);

        let grouped_expression_mactch = {
            // Start tracking your child
            self.dive();
            // `self.dive()` MUST be called here because it mutates `self.pattern_index_sequence`
            // which is used to find associated entry (in self.backtrack_table) of this group itself
            // Thus calling `self.dive()` before computing `match_bound` makes the search
            // in `self.backtrack_table` always fail

            match quantifier {
                Quantifier::None => {
                    // Matching `(E)`
                    // return whatever expression `E` returns
                    self.compute_match()
                }

                Quantifier::ZeroOrOne => {
                    // Matching `(E)?`
                    match self.compute_match() {
                        Some(inner_expression_match) => {
                            if inner_expression_match.end >= self.match_bound {
                                // Match bound exceeded/reached, abort
                                Option::<Match>::None
                            } else {
                                Some(inner_expression_match)
                            }
                        }
                        None => self.empty_expression_match(),
                    }
                }

                _ => {
                    // Matching `(E)*` or `(E)+`

                    // A guard to stop matching if inner expression matched the empty string at least once
                    // so that Matcher does not loop endlessly matching the empty string at current position
                    let mut matched_empty_string = false;

                    let start = self.current();
                    let mut end = self.current();
                    // Keep matching inner expression unless match bound is exceeded
                    // or the inner expression matched the empty string at least once
                    while let Some(new_match) = self.compute_match() {
                        if self.pos > self.match_bound {
                            // Match bound exceeded while matching inner expression
                            // Roll back to end of most recent successful match
                            self.set_position(end);
                            break;
                        }
                        if new_match.is_empty() && matched_empty_string {
                            // Stop matching inner expression E when it has
                            // matched the empty string
                            // or Matcher will never stop because it can always
                            // match the empty string at current position
                            break;
                        }

                        // New match made without exceeding match bound
                        // AND the empty string was NOT matched
                        // Update match end index of this group expression
                        end = new_match.end;
                        matched_empty_string = new_match.is_empty();
                    }

                    // Matched empty range BUT that empty range is NOT the empty string
                    // In other words, failed to match even the empty string
                    if start == end && !matched_empty_string {
                        // Total failure
                        if matches!(quantifier, Quantifier::OneOrMore) {
                            Option::<Match>::None
                        } else {
                            self.empty_expression_match()
                        }
                    } else {
                        // Matched some string, possibly the empty string
                        Some(Match { start, end })
                    }
                }
            }
        };

        self.match_bound = old_match_bound;
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

    // Return Option::<std::ops::Range>::Some(...) on success
    // Return Option::<std::ops::Range>::None on failure
    fn alternation_match(&mut self) -> Option<Match> {
        // Start tracking your children
        self.dive();

        let old_position = self.current();
        let old_pattern = self.pattern.clone();

        let alternation_match = {
            let children = Arc::clone(&old_pattern);
            let children = children
                .read()
                .unwrap()
                .children
                .read()
                .unwrap()
                .iter()
                .map(Arc::clone)
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
                    // That is, this functions cleans up its mess after failing to match
                } else {
                    // One of the branches matched
                    // The whole alternation expression has matched
                    // Return that child match
                    break;
                }

                // Current child failed to match
                // Start tracking next child
                self.appoint_next_child();
            }

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
    // Match one child after another in order

    // Let E be any child of the concatenation expression

    // If E can backtrack and its has backtracked to start of its last successful match
    // then reset its entry in `self.backtrack_table`

    // Attempt to match subexpression E
    // If match succeeded proceed to match next sibling of E

    // If match failed:
    // check if has at least one preceeding sibling which can backtrack
    // and also it has NOT backtracked to its last successful match end

    // If there is no such sibling then the whole expression fails

    // If there IS AT LEAST one sibling which can backtrack
    // set Matcher position to last successful match start position of that sibling
    // and roll back to that sibling and begin matching again towards the end again

    // Repeat this procees until the last child succeeds
    // or the first subexpression which can backtrack fails to match

    // Return Option::<std::ops::Range>::Some(...) on success
    // Return Option::<std::ops::Range>::None on failure
    fn concatenation_match(&mut self) -> Option<Match> {
        // Start tracking your children
        self.dive();

        let old_position = self.current();
        let old_pattern = self.pattern.clone();

        let concatenation_match = {
            let children = Arc::clone(&old_pattern);
            let children = children.read().unwrap();
            let children = children.children.read().unwrap();
            let mut children = children
                .iter()
                .map(|arc| {
                    // (expression, backtrack table (self.backtrack_table) associated entry index)
                    (arc, Option::<usize>::None)
                })
                .collect::<Vec<_>>();

            let mut match_region_end = self.current();
            let mut child_index = 0usize;

            while child_index < children.len() {
                let (child, table_info_pos) = {
                    let child_entry = &mut children[child_index];
                    (child_entry.0.clone(), child_entry.1)
                };

                // First preceeding sibling which can backtrack
                // AND also has NOT backtracked to its last successful match start
                let prev = children[0..child_index]
                    .iter()
                    .enumerate()
                    .filter(|(idx, (_, table_entry))| {
                        let is_preceeding = *idx < child_index;
                        let supports_backtracking = table_entry.is_some();
                        let can_backtrack_again = supports_backtracking
                            && !self.backtrack_table[table_entry.unwrap()]
                                .backtracked_to_last_match_start;
                        is_preceeding && can_backtrack_again
                    })
                    .map(|(idx, (_, table_entry))| (idx, table_entry))
                    .next_back();

                self.pattern = child.clone();
                if let Some(table_pos) = table_info_pos {
                    // Rust won't allow (self.current()) after (&mut self.backtrack_table)
                    let cur = self.current();
                    let table_entry = &mut self.backtrack_table[table_pos];
                    if prev.is_some() && table_entry.backtracked_to_last_match_start {
                        // This expression backtracked all the way back to start
                        // of its last successful match and it has
                        // a preceeding sibling which can backtrack
                        // Reset its entry in `self.backtrack_table`
                        // to make it usable again
                        table_entry.last_match_start = cur;
                        table_entry.last_match_end = self.target.len();
                        table_entry.backtracked_to_last_match_start = false;
                    }
                }

                // Attempt to match current child
                match self.compute_match() {
                    Some(child_match) => {
                        // Child match succeeded

                        // Record its end index
                        match_region_end = child_match.end;

                        // Store backtrack info if this child supports backtracking
                        let table_entry_index = &mut children[child_index].1;
                        if table_entry_index.is_none() && Self::supports_backtracking(&self.pattern)
                        {
                            // Store backtrack info entry index of this expression
                            let table_pos = self
                                .backtrack_table
                                .binary_search_by(|item| {
                                    item.index_sequence.cmp(&self.pattern_index_sequence)
                                })
                                .unwrap();
                            *table_entry_index = Some(table_pos);
                        }
                    }
                    None => {
                        // Child match failed

                        // Check first preceeding sibling which can backtrack
                        // AND has NOT backtracked to its last successful match start
                        match prev {
                            Some((child_idx, table_entry_idx)) => {
                                // Let processing resume from that sibling
                                child_index = child_idx;

                                let table_entry = {
                                    let table_entry_index = table_entry_idx.unwrap();
                                    &self.backtrack_table[table_entry_index]
                                };
                                // Resume matching from the last successful match start of that sibling
                                self.set_position(table_entry.last_match_start);
                                // Fix subexpressions tracker
                                *self.pattern_index_sequence.last_mut().unwrap() = child_index;
                                continue;
                            }
                            None => {
                                // An item failed to match and none of its
                                // preceeding siblings can backtrack
                                // The whole concatenation expression fails

                                // Restore parent pattern to process remaining siblings of parent pattern
                                self.pattern = old_pattern;

                                // Restore old position
                                self.set_position(old_position);

                                // Abandon your children
                                self.bubble_up();

                                return Option::<Match>::None;
                            }
                        }
                    }
                }

                child_index += 1;
                // Start tracking next child
                self.appoint_next_child();
            }

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

impl Iterator for Matcher {
    type Item = Match;

    // Find the next match (non-overlapping with previous match)
    fn next(&mut self) -> Option<Match> {
        // Return Option::<std::ops::Range>::Some(...) on success
        // Return Option::<std::ops::Range>::None on failure

        if matches!(self.next_match_phase, MatchPhase::Finished) {
            // Target is completely consumed
            // No more matches to compute
            return Option::<Match>::None;
        }

        if let Some(cached_range) = { self.match_cache.iter().find(|m| self.pos <= m.start) } {
            let accept_cache = match self.next_match_phase {
                MatchPhase::Normal => true,
                MatchPhase::TrailingEmptyString => cached_range.is_empty(),
                MatchPhase::Finished => false,
            };

            if accept_cache {
                let old_pos = self.pos;
                self.pos = cached_range.end;

                self.next_match_phase = if self.pos < self.target.len() {
                    MatchPhase::Normal
                } else if old_pos < self.target.len() {
                    MatchPhase::TrailingEmptyString
                } else {
                    MatchPhase::Finished
                };

                return Some(cached_range.clone());
            }

            self.next_match_phase = MatchPhase::Finished;
            return Option::<Match>::None;
        }

        // Track root expression
        self.dive();

        // WHY WE NEED A LOOP?
        // Because first match in target string may not start at index 0
        // and hence we need to keep matching until we hit the
        // first successful match or reach end of target
        let mut match_attempt;
        loop {
            match_attempt = self.compute_match();
            // Remove old backtrack info
            self.backtrack_table.clear();
            if match_attempt.is_none() {
                // Last match failed
                if self.has_next() {
                    // Move forward to retry
                    // ADVANCE
                    self.advance();
                } else {
                    // No more characters to process
                    // STOP
                    break;
                }
            } else {
                // Return matched region
                let match_attempt = match_attempt.clone().unwrap();
                if match_attempt.is_empty() {
                    // Matched the empty string in current position
                    // Matcher MUST advance or it will loop endlessly
                    // matching the empty string at the same position
                    // because the empty expression can match anywhere
                    self.advance();
                }

                self.match_cache.insert(
                    self.match_cache
                        .partition_point(|m| match_attempt.start > m.start),
                    match_attempt.clone(),
                );

                if self.matches_substring_start.is_none() {
                    self.matches_substring_start = Some(match_attempt.start);
                }
                self.matches_substring_end = match_attempt.end;

                break;
            }
        }

        self.next_match_phase = match self.pos.cmp(&self.target.len()) {
            std::cmp::Ordering::Less => MatchPhase::Normal,
            _ => match self.next_match_phase {
                MatchPhase::Normal => MatchPhase::TrailingEmptyString,
                _ => MatchPhase::Finished,
            },
        };

        // Abandon root expression
        self.bubble_up();

        match_attempt
    }
}

// Useful methods
impl Matcher {
    // Does some range within the target matches pattern?
    pub fn is_matching(&mut self) -> bool {
        match self.next() {
            None => {
                // May be Matcher consumed itself
                // Retry from the very start
                self.reset();
                self.next().is_some()
            }
            Some(_) => true,
        }
    }

    // Return true if the whole target fully matches pattern
    // In other words, there is exactly one match starting from index 0
    // ending at index N where N is target length
    pub fn fullmatch(&mut self) -> bool {
        self.reset();
        match self.next() {
            Some(m) => m.start == 0 && m.end == self.target.len(),
            None => false,
        }
    }

    // Split target `splits_count` times
    // A large splits_count splits the whole target
    pub fn splitn(&mut self, splits_count: usize) -> Vec<String> {
        if splits_count == 0 {
            return vec![];
        }

        self.reset();
        let target = self.target.iter().collect::<String>();
        let mut splits = vec![];
        let mut split_start = 0;
        for m in self.by_ref() {
            if splits.len() < splits_count {
                splits.push(target[split_start..m.start].to_string());
                split_start = m.end;
            }
        }
        splits.push(target[split_start..target.len()].to_string());

        splits
    }

    // Split the whole target
    pub fn split(&mut self) -> Vec<String> {
        self.splitn(self.target.len() + 1)
    }

    // Return copy of target with `subs_count` substitutions replacing
    // each match with `repl`
    pub fn subn(&mut self, repl: &str, mut subs_count: usize) -> String {
        let target = self.target.iter().collect::<String>();
        if subs_count == 0 {
            return target;
        }

        let mut result = String::with_capacity(self.target.len() + subs_count * repl.len() + 1);
        let mut split_start = 0;
        for m in self.by_ref() {
            if subs_count > 0 {
                result.push_str(&target[split_start..m.start]);
                result.push_str(repl);
                split_start = m.end;
                subs_count -= 1;
            } else {
                break;
            }
        }
        result.push_str(&target[split_start..]);

        result
    }

    // Return copy of target with each match replaced with `repl`
    pub fn sub(&mut self, repl: &str) -> String {
        self.subn(repl, self.target.len() + 1)
    }
}
