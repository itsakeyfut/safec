void *malloc(int n);
void f(int ***** _Nonnull out) {
    int ***b = malloc(8);
    if (!b) return;
    int **c = malloc(8);
    if (!c) return;
    *b = c;
    int ****t = *out;
    if (!t) return;
    *t = b;
    int *x = *c;
}
