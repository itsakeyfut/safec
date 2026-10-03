void *malloc(int n);
void free(void *p);
int h(int *x);

int main(void) {
    int **t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    p[0] = 1;
    r[0] = 1;
    *t2 = p;
    int n = h(r);
    free(r);
    int *q = *t2 + n;
    if (q == 0) {
        return 0;
    }
    return *q;
}
