#if 1 == 1
int by_no_break_space;
#endif
#if 0᠎|| 1
int by_mongolian_vowel_separator;
#endif
#include "white_space_outside_ascii_in_a_condition_or_an_include_is_white_space.h"
int main(void) { return from_header + by_no_break_space + by_mongolian_vowel_separator; }
