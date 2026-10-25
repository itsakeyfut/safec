void *malloc(int n);

int main(void) {
    int *p = malloc(16);
    if (p == 0) {
        return 1;
    }
    int *v = p;
    int **pv = &v;
    int ***ppv = &pv;
    v = p + 4;
    ***ppv = 1;
    return 0;
}
