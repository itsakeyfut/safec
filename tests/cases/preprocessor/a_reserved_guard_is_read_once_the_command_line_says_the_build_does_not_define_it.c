void *malloc(int n);
void free(void *p);
#include "a_reserved_guard_is_read_once_the_command_line_says_the_build_does_not_define_it.h"

int main(void) {
    int *p = malloc(4);
    free(p);
#ifndef SINGLE_THREADED
    free(p);
#endif
    return 0;
}
