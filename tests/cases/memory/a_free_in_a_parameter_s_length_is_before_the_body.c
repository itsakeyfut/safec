void free(void *p);

int f(int * _Nonnull p, int a[(free(p), 1)]) {
    return *p;
}
