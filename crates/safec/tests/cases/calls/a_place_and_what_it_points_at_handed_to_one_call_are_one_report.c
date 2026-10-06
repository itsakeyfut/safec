void *malloc(int n);
void free(void *p);
int two(int **pp, int *q);

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
    free(a);
    return two(*k, **k);
}
