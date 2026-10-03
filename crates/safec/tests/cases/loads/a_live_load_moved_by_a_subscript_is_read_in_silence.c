void *malloc(int n);
void free(void *p);

int main(void) {
    int ***t3 = malloc(8);
    if (t3 == 0) {
        return 0;
    }
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
    *t3 = t2;
    int i = 0;
    p[0] = 2;
    int *q = t3[i][i];
    if (q == 0) {
        return 0;
    }
    return *q;
}
