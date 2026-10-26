int main(void) {
    int *q;
    int **pq = &q;
    int *r = *pq;
    if (r == 0) {
        return 1;
    }
    *r = 1;
    return 0;
}
