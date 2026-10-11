void free(void *p);

int f(int * _Nonnull p, int * _Nonnull q, int b[*q], int a[(q = p, free(p), 1)]) {
    return 0;
}
