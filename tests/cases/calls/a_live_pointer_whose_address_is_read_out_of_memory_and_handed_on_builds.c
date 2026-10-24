void *malloc(int n);
int use2(int **pp);

int f(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    int ***k = malloc(8);
    if (k == 0) {
        return 0;
    }
    *k = &a;
    return use2(*k);
}
