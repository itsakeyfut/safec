void *malloc(int n);
void free(void *p);
#include "a_reserved_guard_is_skipped_once_the_command_line_says_the_build_defines_it.h"

int main(void) {
    int *p = malloc(4);
    free(p);
#ifndef SINGLE_THREADED
    free(p);
#endif
    return 0;
}
