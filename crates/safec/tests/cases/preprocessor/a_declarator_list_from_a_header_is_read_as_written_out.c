void *malloc(int n);
void free(void *p);

int f(void) {
    int
#include "a_declarator_list_from_a_header_is_read_as_written_out.h"
    ;
    a = malloc(4);
    b = a;
    free(a);
    return *b;
}
