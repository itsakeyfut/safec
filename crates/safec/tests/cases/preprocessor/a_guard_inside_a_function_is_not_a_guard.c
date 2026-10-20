void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
#ifndef _MT
#define _MT 1
#else
    free(p);
#endif
    return 0;
}
