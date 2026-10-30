int c(void);
int main(void) {
    int x = 1;
    int *q;
    int **pq = &q;
    if (c()) {
        q = &x;
    }
    for (int i = 0; i < 2; i++) {
        x = x + 1;
    }
    int *r = *pq;
    return r == 0;
}
