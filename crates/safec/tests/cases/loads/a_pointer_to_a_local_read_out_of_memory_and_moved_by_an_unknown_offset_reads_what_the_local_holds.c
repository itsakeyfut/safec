void *malloc(int n);
void free(void *p);
int f(int k) {
    int ***box = malloc(8);
    if (box == 0) {
        return 0;
    }
    int *slot = 0;
    int **po = &slot;
    *box = po;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    slot = a;
    free(a);
    int *q = (*box)[k];
    if (q == 0) {
        return 0;
    }
    return *q;
}
