void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
#ifdef __STDC_VERSION__
    free(p);
#endif
    return 0;
}
#ifndef _Mine
int g(void);
#endif
#define __MINE
#ifdef __MINE
int h(void);
#endif
#ifdef _lower
int g(void);
#endif
