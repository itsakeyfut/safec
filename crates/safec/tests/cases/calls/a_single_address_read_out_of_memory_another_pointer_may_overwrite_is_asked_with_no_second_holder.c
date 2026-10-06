void *malloc(int n);
void grow(int **pp);
int use2(int **pp);

int f(int *** _Nonnull other, int **x) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int ***k = malloc(8);
    if (k == 0) {
        return 0;
    }
    *k = &a;
    *other = x;
    int **q = *k;
    grow(q);
    return use2(&a);
}
