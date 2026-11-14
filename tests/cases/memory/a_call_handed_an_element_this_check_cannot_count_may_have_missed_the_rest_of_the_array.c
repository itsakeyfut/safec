// A call this check cannot read, handed the address of an element it
// cannot count, `&arr[k]`, is handed one element of the array: it may have
// written that one and missed the rest, so `arr[0]` is `SC0411` (#951). At
// a counted element, `&arr[1]`, the element before it is not handed and is
// `SC0411` too. At the array's start the whole array is handed, and the
// call is believed to write it (#782).
void fill(int **p, int *x);

int uncounted(int k) {
    int x = 0;
    int *arr[2];
    int **p = &arr[k];
    fill(p, &x);
    return arr[0] == 0;
}

int counted(void) {
    int x = 0;
    int *arr[2];
    fill(&arr[1], &x);
    return arr[0] == 0;
}

int at_the_start(void) {
    int x = 0;
    int *arr[2];
    fill(arr, &x);
    return arr[0] == 0;
}
