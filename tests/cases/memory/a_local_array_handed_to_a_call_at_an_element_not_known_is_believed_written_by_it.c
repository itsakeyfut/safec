// A call is believed to write what it is handed (#782), so an array handed
// at an element this check cannot name is not doubted after it, where a
// store at such an element is a may-write (ADR-0094 rule 3).
void fill(int **p);
int f(int k) {
    int *arr[2];
    int **p = arr;
    p = p + k;
    fill(p);
    int *r = arr[0];
    return r == 0;
}
