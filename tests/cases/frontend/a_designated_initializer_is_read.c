struct P {
    int x, y;
};
struct P member = {.y = 1};
int index[3] = {[2] = 1, [0] = 3};
struct L {
    struct P a[2];
} nested = {.a[1].y = 4};
