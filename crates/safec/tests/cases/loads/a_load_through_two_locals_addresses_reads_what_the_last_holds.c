void *malloc(int n);
void free(void *p);

int main(void) {
    int *slot = 0;
    int **t2 = &slot;
    int ***t3 = &t2;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    p[0] = 1;
    slot = p;
    free(p);
    int **u = *t3;
    if (u == 0) {
        return 0;
    }
    int *q = *u;
    if (q == 0) {
        return 0;
    }
    return *q;
}
