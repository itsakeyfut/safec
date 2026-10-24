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
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int *b = a;
    *q = &a;
    *q = &b;
    grow_all();
    return use2(&a);
}
