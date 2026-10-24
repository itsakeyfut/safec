void *malloc(int n);
void *realloc(void *p, int n);
void free(void *p);

int main(void) {
    int **pp = malloc(8);
    if (pp == 0) {
        return 0;
    }
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    *pp = a;
    int *q = realloc(*pp, 8);
    if (q == 0) {
        return 0;
    }
    *pp = q;
    int *r = *pp;
    free(pp);
    if (r == 0) {
        return 0;
    }
    int v = r[0];
    free(r);
    return v;
}
