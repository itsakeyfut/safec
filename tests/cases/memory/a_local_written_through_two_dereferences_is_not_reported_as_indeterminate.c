int main(void) {
    int x = 0;
    int *q;
    int **pq = &q;
    int ***ppq = &pq;
    **ppq = &x;
    int *r = *pq;
    return r == 0;
}
