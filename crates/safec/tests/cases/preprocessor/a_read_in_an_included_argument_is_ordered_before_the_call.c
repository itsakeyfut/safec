int g(int x);

int f(int *p) {
    if (p == 0) {
        return 0;
    }
    return g(
#include "a_read_in_an_included_argument_is_ordered_before_the_call.h"
    );
}
