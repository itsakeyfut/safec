int f(int *p) {
#include "an_include_whose_line_ends_in_a_splice_is_refused.h" // \
    if (p == 0) return 0;
    return *p;
}
