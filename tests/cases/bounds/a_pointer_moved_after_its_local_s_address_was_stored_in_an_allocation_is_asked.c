void *malloc(int n);

int main(void) {
    int *p = malloc(16);
    if (p == 0) {
        return 1;
    }
    int ***m = malloc(8);
    if (m == 0) {
        return 1;
    }
    int *v = p;
    *m = &v;
    v = p + 4;
    ***m = 1;
    return 0;
}
