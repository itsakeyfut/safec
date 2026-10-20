void free(void *p);
#define KILL (q = p, free(p), 1)

int f(int * _Nonnull p, int * _Nonnull q, int b[*q], int a[KILL]) {
    return 0;
}
