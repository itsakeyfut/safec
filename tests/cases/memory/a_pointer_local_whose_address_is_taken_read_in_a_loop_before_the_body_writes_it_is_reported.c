int main(void) {
    int *q;
    int **pq = &q;
    int x = 1;
    int *r = 0;
    for (int i = 0; i < 2; i++) {
        r = q;
        *pq = &x;
    }
    return r == 0;
}
