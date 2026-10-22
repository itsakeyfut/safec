void *malloc(int n);
void free(void *p);

int f(void) {
    int *q = malloc(4);
    free(q);
    int y = *q,
#include "a_sibling_declarators_read_beside_a_store_from_a_header_is_reported.h"
    = 0;
    return y;
}
