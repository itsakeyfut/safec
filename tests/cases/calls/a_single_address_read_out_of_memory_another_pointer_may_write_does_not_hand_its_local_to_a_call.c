void *malloc(int n);
void grow(int **pp);
int use2(int **pp);

int f(int *** _Nonnull other) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    int ***k = malloc(8);
    if (k == 0) {
        return 0;
    }
    *k = &a;
    *other = &b;
    int **q = *k;
    grow(q);
    return use2(&a);
}
