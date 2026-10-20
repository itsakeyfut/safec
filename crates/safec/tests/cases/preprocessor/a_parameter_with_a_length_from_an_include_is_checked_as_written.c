void free(void *p);

int f(int * _Nonnull p, int * _Nonnull q,
#include "a_parameter_with_a_length_from_an_include_is_checked_as_written.h"
    , int a[(q = p, free(p), 1)]) {
    return 0;
}
