void *malloc(int n);
void free(void *p);

int f(int * _Nonnull p, int * _Nonnull q,
#include "a_parameter_with_a_length_after_one_without_from_a_header_is_read_as_written_out.h"
    , int a[(q = p, free(p), 1)]) {
    return n;
}
