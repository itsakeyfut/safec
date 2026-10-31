void put_any(void *p);
void run(void) {
    int ***v = 0;
    put_any(&v);
    if (v != 0) {
        int **q = *v;
        if (q != 0) {
            int *r = *q;
            if (r != 0) {
                *r = 1;
            }
        }
    }
}
