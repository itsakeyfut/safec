void *malloc(int n);
void free(void *p);

int main(void) {
    int **t4 = malloc(8);
    if (t4 == 0) {
        return 0;
    }
    int *o = 0;
    int **po = &o;
    int k = 0;
    while (k < 2) {
        int *p = malloc(4);
        if (p == 0) {
            return 0;
        }
        p[0] = 1;
        if (k == 1) {
            int *q = po[k - 1];
            if (q == 0) {
                return 0;
            }
            return *q;
        }
        o = p;
        free(p);
        k = k + 1;
    }
    return 0;
}
