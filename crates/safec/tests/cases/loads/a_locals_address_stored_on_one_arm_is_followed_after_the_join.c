void *malloc(int n);
void free(void *p);

int main(int c) {
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    r[0] = 1;
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
    int *slot = 0;
    int **t2 = &slot;
    if (c) {
        *t3 = t2;
    }
    **t3 = r;
    free(r);
    return ***t3;
}
