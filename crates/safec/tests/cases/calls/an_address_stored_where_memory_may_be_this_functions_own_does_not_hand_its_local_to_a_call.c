void *malloc(int n);
void release_ref(int **pp);
int use2(int **pp);

int f(int **** _Nonnull tt, int c) {
    int ***s = malloc(8);
    if (s == 0) {
        return 0;
    }
    int ***k = s;
    if (c) {
        k = *tt;
    }
    if (k == 0) {
        return 0;
    }
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    *k = &a;
    release_ref(&b);
    return use2(&a);
}
