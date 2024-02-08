#[allow(dead_code)]
pub mod tokens;

use tokens::{Token, TokenName::*};

// what kind of balanced characters "(){}[]"
// currently scanned character are between
pub enum GroupingTag {
    // ( and )
    GroupParentheses,
    // more comin' . . .
}

pub struct GroupingMark {
    // what kind of balanced characters this mark refers to
    tag: GroupingTag,
    // index of left-hand balanced character "( or { or ["
    position: usize,
}

pub struct Scanner {
    // source string characters vector to allow fast access
    source: Vec<char>,
    // index of character in field (source) that's currenlty processed
    current: usize,
    // `found_empty_string` flag indicates whether we found the empty string token
    // in current position
    // when it's false it means we can attempt to generate EmptyString token
    // when it's true it means we already generated EmtpyString token or we could not do so
    // rather we should attempt to generate another token (if any remaining)
    found_empty_string: bool,
    // Type and start position of each grouping construct
    // which are: groups (...) | character classes [...] | length modifiers {m, n}
    // we use Vec because group expression can nest
    // even though {} and [] do not
    groupings: Vec<GroupingMark>,
}

// an Iterator transforming source string into a tokens stream
// each toekn is generated on request
impl Scanner {
    pub fn new(source: &str) -> Scanner {
        // source characters as a vector for fast access
        let source = source.chars().collect::<Vec<_>>();
        // current (`processed` or `to be processed`) character
        let current = 0;
        // flag (found_empty_string) is false on start
        // because the empty string can occur anywhere with an abitrary string
        // even within the empty string (which is itself)
        let found_empty_string = false;
        // grouping constructs marks stack
        // we need a stack because grouped expressions `(...)` can nest
        let groupings = vec![];
        Scanner {
            source,
            current,
            found_empty_string,
            groupings,
        }
    }

    // construct source string from field (self.source)
    pub fn get_source_string(&self) -> String {
        // pre-allocate at least `self.source.len()` bytes
        // to make appending characters faster
        self.source.iter().collect::<String>()
    }

    // get character at (index + offset) if this position exists
    // otherwise return \0
    fn get(&self, index: usize, offset: isize) -> char {
        let negative = offset < 0;
        // absolute value of `offset` as a usize integer
        let offset = offset.unsigned_abs();
        if negative {
            if index < offset {
                // index < |offset| ===> index - |offset| < 0
                // we return \0 to indicate the absence of characters
                // at this (negative) index (which is invalid)
                return '\0';
            }
            // index >= |offset| ===> index - |offset| >= 0
            // index - |offset| is valid index but still it may be out of bound
            // thus call Vec::get to get an Option which you can handle
            // if returned Option is None
            return *self.source.get(index - offset).unwrap_or(&'\0');
        }
        // index + offset a valid index but still it may be out of bound
        // again use Vec::get to avoid out-of-bound indexing
        *self.source.get(index + offset).unwrap_or(&'\0')
    }

    // advance the current character marker one step forward
    fn advance(&mut self) {
        self.current += 1;
    }

    // check if we reached end of input
    // if current character marker index is a valid index
    // then we still have characters to process
    // otherwise we reached end of input
    fn has_next(&self) -> bool {
        self.current < self.source.len()
    }

    // get character right before currently processed character
    fn previous(&self) -> char {
        self.get(self.current, -1)
    }

    // get the currenlty processed character
    fn peek(&self) -> char {
        self.get(self.current, 0)
    }

    // get character right after currently processed character
    fn next_char(&self) -> char {
        self.get(self.current, 1)
    }
}

impl Iterator for Scanner {
    type Item = Token;

    // (Attempt to) generate a token for the current character
    // or an EmptyString token
    fn next(&mut self) -> Option<Token> {
        // First, try to generate an EmptyString token because
        // the empty string can appear anywhere within a string
        // even within the empty string (which is itself)
        let peek = self.peek();

        let prev = self.previous();
        // if certain characters "( | )" are adjacent with the former not escaped
        // we can generate an EmptyString token
        // self.source[self.current - 2] (if exists) is second to current character
        // it's where to look to check whether previous character is preceeded by a slash
        // which means it's escaped
        let is_prev_escaped = self.get(self.current, -2) == '\\';
        if !is_prev_escaped && !self.found_empty_string {
            // Set flag (self.found_empty_string) to not attempt to generate EmptyString token
            // if previous iteration did
            self.found_empty_string = true;
            // There are 3 cases in which there is an
            // empty string token between metacharacters
            // CASE 1: An empty source string or a string starting with |
            if (self.source.is_empty() || (self.source.len() == 1 && peek == '|'))
                // CASE 2: ( followed by either | or )
                || (prev == '(' && (peek == '|' || peek == ')'))
                // CASE 3: `| followed by ) or another |` or `| is the last character in input`
                || ((prev == '|' && (peek == ')' || peek == '|')) || (peek == '|' && self.current+1 == self.source.len()))
            {
                // Note that we do not call advance()
                // because EmptyString contains no characters at all
                // and hence we never actually moved
                // instead we set flag (found_empty_string) so
                // next time call `next` we do not visit this branch again
                return Some(Token {
                    name: EmptyString,
                    position: self.current,
                });
            }
            // we did not generate an EmptyString token at current position
            // but none of the three above cases occurred
            // we try to generate another token (if any remaining)
        }

        // Try to generate EmptyString token when calling `next` again
        self.found_empty_string = false;
        // note that even if flag (found_empty_string) was unset before calling Iterator::next
        // if execution reached to region of code then the return value of this call to
        // Iterator::next must return an Option::Some and then advancing
        // or Option::None which means we reached end of input
        // in both cases Scanner will NOT attempt to generate an EmptyString token
        // twice at the same position, we can't get stuck in a loop

        // When scanner is given an empty string as input
        // it generates an EmptyString token but self.current is still 0
        // when calling `next` again, it can NOT generate another EmptyString token
        // because flag `found_empty_string` is set by then
        // hence it reaches this region of code
        // the call `self.has_next()` performs the comparison
        // self.current(which is 0) < self.source.len() (also 0) which is 0 < 0
        // clearly false and then `return None` executes signaling the end of iterator
        if !self.has_next() {
            // We reached end of input and we can not generate
            // another token, not even EmptyString

            // But we need to check for un-balanced ( before quitting
            if !self.groupings.is_empty() {
                // Place a caret `^` below each un-balanced (
                // we can retrieve from field (self.groupings)

                // String containing a caret aligned with each un-balanced (
                // pre-allocate at least `self.source.len()` bytes
                // to make appending characters faster
                let mut error_indicator = String::with_capacity(self.source.len());
                for mark in &self.groupings {
                    while error_indicator.len() < mark.position {
                        // add spaces fill for alignment
                        error_indicator.push(' ');
                    }
                    // add error indicator `^`
                    error_indicator.push('^');
                }
                // re-construct source string
                let source = self.get_source_string();
                eprintln!(
                    "Error: Un-balanced characters\n\
                    {source}\n{error_indicator}"
                );
                // we could called std::process::exit, but panicing allows
                // to find code generating the error through backtrace provide by panic!
                panic!();
            }

            // All characters are consumed and we can not generate an EmptyString token
            // this iterator has no more elements, return None
            return None;
        }

        // By default assume the current character is an ordinary character
        // (not a metacharacter and not an escaped metacharacter)
        let mut next = Some(Token {
            name: Character { value: peek },
            position: self.current,
        });
        // a mutable (&mut) reference to Token object inside local variable `next`
        // we use this &mut reference to modify Token::name field in case current character
        // is not an ordinary character (metacharacter or an escaped metacharacter)
        let next_token = next.as_mut().unwrap();

        match peek {
            '(' => {
                // Mark this position as the beginning of a group expression `(...)`
                self.groupings.push(GroupingMark {
                    tag: GroupingTag::GroupParentheses,
                    position: self.current,
                });
                next_token.name = LeftParen;
            }
            ')' => {
                if self.groupings.is_empty() {
                    // Error: Un-balanced )

                    // a string of spaces ending with a `^`
                    // aligned with source string to indicate the un-balanced )
                    // pre-allocate at least `self.source.len()` bytes
                    // to make appending characters faster
                    let mut error_indicator = String::with_capacity(self.source.len());
                    while error_indicator.len() < self.current {
                        // add spaces fill for alignment
                        error_indicator.push(' ');
                    }
                    // add `^` to indicat the un-balanced )
                    error_indicator.push('^');
                    // re-construct source string
                    let source = self.get_source_string();
                    let error_position = self.current;
                    eprintln!(
                        "Error in position {error_position}: Un-balanced )\n\
                        {source}\n{error_indicator}"
                    );
                    // panic! to use backtrace if needed
                    panic!();
                }
                // Remove most recently appended marker to indicate
                // end of most recently scanned group
                self.groupings.pop();
                next_token.name = RightParen;
            }
            '|' => {
                next_token.name = Pipe;
            }
            '?' => {
                next_token.name = Mark;
            }
            '*' => {
                next_token.name = Star;
            }
            '+' => {
                next_token.name = Plus;
            }
            '.' => {
                next_token.name = Dot;
            }
            '\\' => {
                let next_char = self.next_char();
                // if this flag is true then we must advanced one more time
                // because we found an escpaed metacharacter which is actually
                // two characters: slash followed by a metacharacter
                // the call to `self.advance()` here is to move to metacharacter
                // following current slash (stored in local field `peek`)
                // the final call to `self.advance()` at the end of Iterator::new
                // moves to next character after the now found metacharacter
                let mut found_escaped_metachar = true;
                match next_char {
                    '\\' => {
                        next_token.name = EscapedSlash;
                    }
                    '(' => {
                        next_token.name = EscapedLeftParen;
                    }
                    ')' => {
                        next_token.name = EscapedRightParen;
                    }
                    '|' => {
                        next_token.name = EscapedPipe;
                    }
                    '?' => {
                        next_token.name = EscapedMark;
                    }
                    '*' => {
                        next_token.name = EscapedStar;
                    }
                    '+' => {
                        next_token.name = EscapedPlus;
                    }
                    '.' => {
                        next_token.name = EscapedDot;
                    }
                    _ => {
                        found_escaped_metachar = false;
                    }
                }
                if found_escaped_metachar && self.has_next() {
                    // the additional condition `self.has_next()` ensures
                    // than `self.current` is never increased if it's already
                    // equal to `self.source.len()`
                    self.advance();
                }
            }
            _ => {
                // Any other ordinary character.
                // that's, not a metacharacter and an escaped metacharacter
                // Nothing to be handled because by default
                // token name is TokenName::Character
            }
        }
        // move current character marker one step forward
        self.advance();
        next
    }
}
