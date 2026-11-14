// A call is believed to write what it is handed (#782), but an array
// handed at an element this check cannot name is handed one element: the
// call may have written it and missed the rest, as a store at such an
// element may (ADR-0094 rule 3), so `arr[0]` is doubted (#951).
void fill(int **p);
int f(int k) {
    int *arr[2];
    int **p = arr;
    p = p + k;
    fill(p);
    int *r = arr[0];
    return r == 0;
}
