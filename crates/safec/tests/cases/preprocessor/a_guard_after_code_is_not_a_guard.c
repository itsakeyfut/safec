void *malloc(int n);
void free(void *p);
#ifndef _MT
#define _MT 1
int f(void) {
    int *p = malloc(4);
    free(p);
    free(p);
    return 0;
}
#endif
