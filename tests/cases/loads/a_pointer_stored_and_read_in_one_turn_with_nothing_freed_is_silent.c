void *malloc(int n);
void free(void *p);

int main(void) {
    int **t2 = malloc(8);
    if (t2 == 0) {
        return 0;
    }
    int k = 0;
    int s = 0;
    while (k < 2) {
        int *p = malloc(4);
        if (p == 0) {
            return 0;
        }
        p[0] = 1;
        *t2 = p;
        int *q = *t2;
        s = s + *q;
        k = k + 1;
    }
    return s;
}
