void *malloc(int n);
void grow_deep(int ***box);
int use2(int **pp);

int f(int *** _Nonnull box) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    *box = &b;
    grow_deep(box);
    return use2(&b);
}
