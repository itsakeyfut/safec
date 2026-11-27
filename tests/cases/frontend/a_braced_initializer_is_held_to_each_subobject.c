struct P {
    int *p;
    int n;
};

int x;
struct P wrong = {1, 2};
int too_many[2] = {1, 2, 3};
struct P nested_too_many = {{&x, 1}};
int empty[2] = {};
char short_name[2][2] = {"abc"};
