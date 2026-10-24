void *malloc(int n);
void free(void *p);
int f(int k) {
    int *slot = 0;
    int **po = &slot;
    int ***ppo = &po;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    slot = a;
    free(a);
    int *q = (*ppo)[k];
    if (q == 0) {
        return 0;
    }
    return *q;
}
