int main(void) {
    int x = 1;
    int *r = 0;
    for (int j = 0; j < 2; j++) {
        int *q;
        int **pq = &q;
        for (int i = 0; i < 2; i++) {
            q = &x;
        }
        r = *pq;
    }
    return r == 0;
}
