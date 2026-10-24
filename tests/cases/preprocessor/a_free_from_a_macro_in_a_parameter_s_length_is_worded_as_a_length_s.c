void free(void *p);
#define KILL (free(p), 1)

int f(int * _Nonnull p, int a[*p + KILL]) {
    return 0;
}
