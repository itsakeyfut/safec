void *malloc(int n);
void grow_all(void);
int use2(int **pp);

int f(int ****tt) {
    if (tt == 0) {
        return 0;
    }
    int ***q = *tt;
    if (q == 0) {
        return 0;
    }
    int *b = malloc(4);
    if (b == 0) {
        return 0;
    }
    *q = &b;
    grow_all();
    return use2(&b);
}
