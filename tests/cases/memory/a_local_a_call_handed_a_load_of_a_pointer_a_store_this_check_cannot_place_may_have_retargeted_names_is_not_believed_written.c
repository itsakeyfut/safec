void stash(int ****a, int *****box);
void fill(int **p);
int f(void) {
    int *m[1];
    int **p = m;
    int **other = 0;
    int ***pp = 0;
    int ****cell = 0;
    stash(&pp, &cell);
    pp = &p;
    int ****t = cell;
    if (t) *t = &other;
    int ***r = pp;
    if (r) {
        fill(*r);
        int *q = m[0];
        return q == 0;
    }
    return 0;
}
